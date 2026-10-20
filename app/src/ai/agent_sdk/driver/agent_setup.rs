//! Shared resource setup for normal runs and standby sandboxes before an agent prompt starts.

use std::ffi::OsStr;
use std::path::PathBuf;

use ai::skills::{
    SKILL_PROVIDER_DEFINITIONS, parse_bundled_skill, read_skills_for_skills_dirs,
    resolve_skills_dirs,
};
use futures::future::join_all;
use itertools::Itertools as _;
use repo_metadata::local_model::IndexedRepoState;
use repo_metadata::{RepoMetadataModel, RepositoryIdentifier};
use warp_cli::agent::Harness;
use warp_cli::mcp::MCPSpec;
use warp_cli::skill::SkillSpec;
use warp_core::features::FeatureFlag;
use warp_core::safe_info;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{Entity, ModelSpawner, SingletonEntity};

use super::environment::{
    FACTORY_REPO_CLONE_URL_ENV_VAR, FACTORY_REPO_DIR_ENV_VAR,
    prepend_factory_definition_clone_for_values,
};
use super::{
    AgentDriver, AgentDriverError, StandbyWorkspace, WorkspaceState, environment, mcp_startup,
};
use crate::ai::agent_sdk::environment_snapshot::{
    EnvironmentSnapshot, EnvironmentSnapshotReporter,
};
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::cloud_environments::{GithubRepo, SourceRepo};
use crate::ai::skills::{
    SkillManager, SkillWatcher, factory_deferred_repositories_skill_path, filter_skills_by_spec,
    read_skills_from_directories, resolve_skill_repos,
};
use crate::auth::AuthStateProvider;
use crate::server::server_api::ServerApiProvider;
use crate::terminal::shell::ShellType;

pub(crate) mod configured_skills;
impl AgentDriver {
    /// Consumes the workspace once to prepare standby resources without starting a prompt.
    #[cfg_attr(all(target_family = "wasm", not(test)), expect(dead_code))]
    pub(crate) async fn prepare_resources(
        foreground: &ModelSpawner<Self>,
        mcp_specs: Vec<MCPSpec>,
        global_skills: GlobalSkillResolution,
        setup_events: &SetupClientEventReporter,
    ) -> Result<PathBuf, AgentDriverError> {
        // Standby resolves global skills before this call to check checkout names before any clone.
        let mut workspace = foreground
            .spawn(|me, _| match me.workspace.take() {
                Some(WorkspaceState::Pending(workspace)) => Some(workspace),
                Some(WorkspaceState::Prepared(_)) | None => None,
            })
            .await?
            .ok_or(AgentDriverError::InvalidRuntimeState)?;
        let prepared_configuration = workspace.clone();
        bootstrap_terminal(foreground, setup_events).await?;
        setup_events
            .record_result(
                SetupStep::CloudProviderSetup,
                Self::setup_cloud_providers(foreground),
            )
            .await?;
        setup_events
            .record_result(
                SetupStep::GlobalSkillRepoClone,
                clone_global_skill_repos(foreground, &global_skills.repos),
            )
            .await?;
        let (workspace, snapshot_reporter, task_id) = foreground
            .spawn(move |me, ctx| {
                let shell_type = me
                    .terminal_driver
                    .as_ref(ctx)
                    .active_session_shell_type(ctx)
                    .unwrap_or(ShellType::Bash);
                let env_value = |name| {
                    me.resolved_env_vars
                        .get(OsStr::new(name))
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                };
                prepend_factory_definition_clone_for_values(
                    env_value(FACTORY_REPO_CLONE_URL_ENV_VAR),
                    env_value(FACTORY_REPO_DIR_ENV_VAR),
                    shell_type,
                    &mut workspace.setup_commands,
                );
                let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client().clone();
                (
                    workspace,
                    EnvironmentSnapshotReporter::noop(ai_client, ctx.background_executor()),
                    me.task_id,
                )
            })
            .await?;
        let prepared = prepare_workspace(
            foreground,
            Harness::Oz,
            workspace,
            setup_events,
            snapshot_reporter,
        )
        .await?;
        let working_dir = prepared.harness_working_dir.clone();
        let repositories = prepared.repositories.clone();
        foreground
            .spawn(move |me, _| {
                me.harness_working_dir = working_dir;
                me.workspace = Some(WorkspaceState::Prepared(StandbyWorkspace {
                    configuration: prepared_configuration,
                    repositories,
                }));
            })
            .await?;
        let managed_mcp_client = foreground
            .spawn(|_, ctx| ServerApiProvider::as_ref(ctx).get_managed_mcp_client())
            .await?;
        let mcp_result = setup_events
            .record_result(
                SetupStep::McpServerStartup,
                mcp_startup::start_task_and_profile_mcp_servers(
                    &mcp_specs,
                    managed_mcp_client,
                    mcp_startup::McpStartupMode::Standby,
                    task_id,
                    foreground,
                ),
            )
            .await;
        Self::handle_mcp_startup_result(mcp_result, foreground).await?;
        load_skills(
            foreground,
            prepared.repositories,
            global_skills,
            setup_events,
        )
        .await;
        Ok(prepared.harness_working_dir)
    }
}

/// Keeps workspace setup results for harness startup and skill discovery.
pub(crate) struct PreparedWorkspace {
    /// Selects the single repository directory, or the root for zero or multiple repositories.
    pub harness_working_dir: PathBuf,
    /// Identifies workspace repositories to scan for skills, excluding deferred repositories.
    pub repositories: Vec<SourceRepo>,
}

/// Waits for shell integration before setup can execute terminal commands.
pub(crate) async fn bootstrap_terminal(
    foreground: &ModelSpawner<AgentDriver>,
    setup_events: &SetupClientEventReporter,
) -> Result<(), AgentDriverError> {
    setup_events
        .record_result(SetupStep::TerminalBootstrap, async {
            foreground
                .spawn(|me, ctx| {
                    me.terminal_driver
                        .update(ctx, |driver, _| driver.wait_for_session_bootstrapped())
                })
                .await?
                .await
                .map_err(|error| AgentDriverError::BootstrapFailed { error })
        })
        .await
}

/// Resolves global skills and attempts their repository clones as part of normal run setup.
pub(crate) async fn prepare_global_skills(
    foreground: &ModelSpawner<AgentDriver>,
    setup_events: &SetupClientEventReporter,
) -> Result<GlobalSkillResolution, AgentDriverError> {
    let skills = setup_events
        .record_result(
            SetupStep::GlobalSkillResolution,
            resolve_global_skills(foreground),
        )
        .await?;
    setup_events
        .record_result(
            SetupStep::GlobalSkillRepoClone,
            clone_global_skill_repos(foreground, &skills.repos),
        )
        .await?;
    Ok(skills)
}

/// Runs workspace setup and returns the harness directory and repositories to scan for skills.
pub(crate) async fn prepare_workspace(
    foreground: &ModelSpawner<AgentDriver>,
    harness: Harness,
    workspace: environment::WorkspaceConfiguration,
    setup_events: &SetupClientEventReporter,
    snapshot_reporter: EnvironmentSnapshotReporter,
) -> Result<PreparedWorkspace, AgentDriverError> {
    let repositories = workspace.source_repos.clone();
    let mut harness_working_dir = foreground.spawn(|me, _| me.working_dir.clone()).await?;
    if workspace.needs_preparation() {
        let setup_events = setup_events.clone();
        harness_working_dir = foreground
            .spawn(move |me, ctx| {
                let working_dir = me.working_dir.clone();
                me.terminal_driver.update(ctx, |_, ctx| {
                    environment::prepare_environment(
                        working_dir,
                        false,
                        harness,
                        workspace,
                        setup_events,
                        snapshot_reporter,
                        ctx,
                    )
                })
            })
            .await?
            .await?;
    } else {
        snapshot_reporter.report(EnvironmentSnapshot::empty());
    }
    Ok(PreparedWorkspace {
        harness_working_dir,
        repositories,
    })
}

/// Loads workspace, global, and configured-directory skills without making load failures fatal.
pub(crate) async fn load_skills(
    foreground: &ModelSpawner<AgentDriver>,
    repositories: Vec<SourceRepo>,
    global_skills: GlobalSkillResolution,
    setup_events: &SetupClientEventReporter,
) {
    setup_events
        .record_value(
            SetupStep::EnvironmentSkillLoading,
            load_environment_skills(foreground, repositories),
        )
        .await;
    setup_events
        .record_value(
            SetupStep::GlobalSkillLoading,
            load_global_skills(foreground, global_skills.specs, global_skills.repos),
        )
        .await;
    setup_events
        .record_value(SetupStep::SkillsDirsLoading, load_skills_dirs(foreground))
        .await;
}

/// Keeps one resolved global skill selection consistent through validation, cloning, and loading.
pub(crate) struct GlobalSkillResolution {
    /// Selects which skills to load from the cloned repositories.
    pub specs: Vec<SkillSpec>,
    /// Lists the repositories needed to supply the selected skills.
    pub repos: Vec<GithubRepo>,
}

/// Reads global skill settings without cloning repositories or requiring an agent driver.
pub(crate) async fn resolve_global_skills<H: Entity>(
    foreground: &ModelSpawner<H>,
) -> Result<GlobalSkillResolution, AgentDriverError> {
    if !FeatureFlag::OzPlatformSkills.is_enabled() {
        return Ok(GlobalSkillResolution {
            specs: Vec::new(),
            repos: Vec::new(),
        });
    }

    let raw_global_specs = foreground
        .spawn(|_, ctx| AuthStateProvider::as_ref(ctx).get().global_skills())
        .await?;
    let (global_specs, global_repos) = resolve_skill_repos(&raw_global_specs);
    if !global_repos.is_empty() {
        log::info!("Resolving {} global skill repo(s)", global_repos.len());
    }
    Ok(GlobalSkillResolution {
        specs: global_specs,
        repos: global_repos,
    })
}

/// Clone all passed-in global skill repositories.
///
/// These repositories are not registered with the `DetectedRepositories`
/// model, so that we don't automatically detect *all* skills they contain.
/// This is important when using registry-like skill repos where loading all
/// skills would bloat the context window.
pub(crate) async fn clone_global_skill_repos(
    foreground: &ModelSpawner<AgentDriver>,
    global_skill_repos: &[GithubRepo],
) -> Result<(), AgentDriverError> {
    if global_skill_repos.is_empty() {
        return Ok(());
    }

    // Global skill specs are still GitHub-only, while the shared environment clone
    // helper accepts provider-neutral repositories. Adapt them only at the clone boundary.
    let global_skill_repos = global_skill_repos
        .iter()
        .map(SourceRepo::from)
        .collect::<Vec<_>>();
    let clone_future = foreground
        .spawn(move |me, ctx| {
            let working_dir = me.working_dir.clone();
            me.terminal_driver.update(ctx, |_, ctx| {
                let spawner = ctx.spawner();
                async move {
                    environment::clone_repos(&global_skill_repos, &working_dir, &spawner).await
                }
            })
        })
        .await?;

    if let Err(err) = clone_future.await {
        log::warn!("Failed to clone one or more global-skill repos: {err}");
    }

    Ok(())
}

/// Load skills from environment repositories.
///
/// It's assumed that `prepare_environment` registers all cloned repositories
/// with the `DetectedRepositories` model, so that we can scan for skills
/// here.
pub(crate) async fn load_environment_skills(
    foreground: &ModelSpawner<AgentDriver>,
    repos: Vec<SourceRepo>,
) {
    if repos.is_empty() {
        log::info!("No environment repositories for skill loading");
        return;
    }
    safe_info!(
        safe: ("Loading skills from {} environment repositories", repos.len()),
        full: (
            "Loading environment skills from repositories: {}",
            repos.iter().join(", ")
        )
    );

    // Skill-scanning depends on the in-memory RepoMetadataModel index, so wait for
    // initial indexing of all repos to complete.
    let repo_index_waits = foreground
        .spawn(move |me, ctx| {
            let repo_paths: Vec<PathBuf> = repos
                .iter()
                .map(|repo| me.working_dir.join(&repo.repo))
                .collect();
            let repo_metadata = RepoMetadataModel::handle(ctx);
            let mut repo_index_waits = Vec::new();
            for repo_path in &repo_paths {
                let Some(id) = RepositoryIdentifier::try_local(repo_path) else {
                    log::warn!(
                        "Cannot wait for repository metadata indexing for non-local path {}",
                        repo_path.display()
                    );
                    continue;
                };
                let wait = repo_metadata.update(ctx, |repo_metadata, ctx| {
                    repo_metadata.repository_indexed(&id, ctx)
                });
                repo_index_waits.push((repo_path.clone(), id, wait));
            }
            (repo_paths, repo_index_waits)
        })
        .await;

    let (repo_paths, repo_index_waits) = match repo_index_waits {
        Ok(result) => result,
        Err(err) => {
            log::warn!("Failed to prepare environment skill loading: {err}");
            return;
        }
    };

    if !repo_index_waits.is_empty() {
        log::info!(
            "Waiting for repository metadata indexing before loading skills from {} repo(s)",
            repo_index_waits.len()
        );
        let (repo_index_targets, wait_futures): (Vec<_>, Vec<_>) = repo_index_waits
            .into_iter()
            .map(|(repo_path, id, wait)| ((repo_path, id), wait))
            .unzip();
        join_all(wait_futures).await;
        let repo_index_statuses = foreground
            .spawn(move |_, ctx| {
                let repo_metadata = RepoMetadataModel::handle(ctx);
                repo_index_targets
                    .into_iter()
                    .filter_map(|(repo_path, id)| {
                        let RepositoryIdentifier::Local(repo_id_path) = &id else {
                            return None;
                        };
                        let error = match repo_metadata.as_ref(ctx).repository_state(&id, ctx) {
                            Some(IndexedRepoState::Indexed(_)) => None,
                            Some(IndexedRepoState::Pending(_)) => Some(format!(
                                "Repository indexing is still pending: {repo_id_path}"
                            )),
                            Some(IndexedRepoState::Failed(error)) => {
                                Some(format!("Repository indexing failed: {error}"))
                            }
                            None => Some(format!("Repository not found: {repo_id_path}")),
                        };
                        Some((repo_path, error))
                    })
                    .collect::<Vec<_>>()
            })
            .await;

        let repo_index_statuses = match repo_index_statuses {
            Ok(repo_index_statuses) => repo_index_statuses,
            Err(err) => {
                log::warn!("Failed to check repository indexing status: {err}");
                Vec::new()
            }
        };
        for (repo_path, error) in repo_index_statuses {
            if let Some(err) = error {
                log::warn!(
                    "Repository metadata indexing was not ready for skill loading in {}: {err}",
                    repo_path.display()
                );
            }
        }
    }

    let load_skills_result = foreground
        .spawn(move |_, ctx| {
            let skills = SkillWatcher::read_local_skills_for_repos(&repo_paths, ctx);
            if !skills.is_empty() {
                log::info!("Loaded {} environment skill(s)", skills.len());
            } else {
                log::info!("No environment skills found");
            }
            SkillManager::handle(ctx).update(ctx, |manager, _| {
                manager.set_cloud_environment(true);
                manager.handle_skills_added(skills);
            });
        })
        .await;

    if let Err(err) = load_skills_result {
        log::warn!("Failed to load environment skills: {err}");
    }
}

/// Load explicitly requested global skills by reading directly from disk.
pub(crate) async fn load_global_skills(
    foreground: &ModelSpawner<AgentDriver>,
    specs: Vec<SkillSpec>,
    repos: Vec<GithubRepo>,
) {
    if specs.is_empty() || repos.is_empty() {
        return;
    }
    safe_info!(
        safe: ("Loading {} global skill(s) from {} repo(s)", specs.len(), repos.len()),
        full: (
            "Loading global skills {} from repos: {}",
            specs.iter().map(|s| &s.skill_identifier).join(", "),
            repos.iter().join(", ")
        )
    );

    let load_result = foreground
        .spawn(move |me, _| {
            let mut all_skills = Vec::new();
            for repo in &repos {
                let repo_path = me.working_dir.join(&repo.repo);
                // Read skills from all known provider directories on disk,
                // without depending on RepoMetadataModel.
                let skill_dirs = SKILL_PROVIDER_DEFINITIONS
                    .iter()
                    .map(|def| repo_path.join(&def.skills_path));
                let repo_skills = read_skills_from_directories(skill_dirs);
                let filtered = filter_skills_by_spec(
                    &LocalOrRemotePath::Local(repo_path),
                    repo_skills,
                    &specs,
                );
                all_skills.extend(filtered);
            }
            all_skills
        })
        .await;

    let skills = match load_result {
        Ok(skills) => skills,
        Err(err) => {
            log::warn!("Failed to load global skills: {err}");
            return;
        }
    };

    if skills.is_empty() {
        log::info!("No global skills matched the requested specs");
        return;
    }

    log::info!("Loaded {} global skill(s)", skills.len());
    let add_result = foreground
        .spawn(move |_, ctx| {
            SkillManager::handle(ctx).update(ctx, |manager, _| {
                manager.set_cloud_environment(true);
                manager.handle_skills_added(skills);
            });
        })
        .await;

    if let Err(err) = add_result {
        log::warn!("Failed to add global skills to SkillManager: {err}");
    }
}

/// Load skills from the `WARP_SKILL_DIRS` environment variable as personal (home) tier skills.
///
/// `WARP_SKILL_DIRS` is a comma-separated list of paths; each entry is itself a skills directory
/// whose **direct children** are expected to be skill folders containing `SKILL.md`. Relative
/// entries are resolved against the host's working directory — not the process's current
/// working directory, which environment preparation may have changed (e.g. by cd-ing into a
/// cloned repo). Skills loaded this way behave identically to `~/.agents/skills` personal
/// skills—always in scope, regardless of the current working directory.
///
/// Invalid, missing, or unreadable entries are skipped with a warning; an unset or empty
/// variable is a no-op.
pub(crate) async fn load_skills_dirs(foreground: &ModelSpawner<AgentDriver>) {
    let dirs = foreground
        .spawn(|me, _| me.skill_dirs.clone())
        .await
        .unwrap_or_default();
    if dirs.is_empty() {
        return;
    }
    log::info!(
        "WARP_SKILL_DIRS: loading skills from {} directories",
        dirs.len()
    );
    let load_result = foreground
        .spawn(move |me, ctx| {
            let dirs = resolve_skills_dirs(&me.working_dir, dirs);
            let skills = read_skills_for_skills_dirs(&dirs);
            if skills.is_empty() {
                log::info!("WARP_SKILL_DIRS: no skills found");
            } else {
                log::info!("WARP_SKILL_DIRS: loaded {} skill(s)", skills.len());
            }
            SkillManager::handle(ctx).update(ctx, |manager, _| {
                manager.add_skills_dirs_skills(skills);
            });
        })
        .await;
    if let Err(err) = load_result {
        log::warn!("Failed to load WARP_SKILL_DIRS skills: {err}");
    }
}

/// Loads the bundled instructions that let an active Factory run discover and clone deferred repos.
pub(crate) async fn load_factory_deferred_repositories_skill(
    foreground: &ModelSpawner<AgentDriver>,
) {
    let Some(path) = factory_deferred_repositories_skill_path() else {
        log::warn!("Bundled deferred repositories skill is unavailable");
        return;
    };
    let skill = match parse_bundled_skill(&path.join("SKILL.md")) {
        Ok(skill) => skill,
        Err(error) => {
            log::warn!("Could not read bundled deferred repositories skill: {error}");
            return;
        }
    };
    if let Err(error) = foreground
        .spawn(move |_, ctx| {
            SkillManager::handle(ctx).update(ctx, |manager, _| {
                manager.add_skills_dirs_skills(vec![skill]);
            });
        })
        .await
    {
        log::warn!("Could not register bundled deferred repositories skill: {error}");
    }
}

#[cfg(test)]
#[path = "agent_setup_tests.rs"]
mod tests;
