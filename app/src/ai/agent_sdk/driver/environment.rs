use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai::index::full_source_code_embedding::manager::{
    CodebaseIndexManager, CodebaseIndexManagerEvent,
};
use chrono::Utc;
use cloud_object_models::CodeForge;
use futures::channel::oneshot;
use futures::future::{Either, join_all, select};
use instant::Instant;
use repo_metadata::repositories::{DetectedRepositories, RepoDetectionSource};
use warp_cli::agent::{
    RepositoryForge, RepositoryHeadRef, RepositoryIdentity, RepositoryPreparationOverride,
};
use warp_cli::environment_checkout::{
    CheckoutBatch, CheckoutFailure, CheckoutFailureKind, CheckoutFailureReport, CheckoutRequest,
};
use warp_completer::completer::{CommandExitStatus, CommandOutput};
use warp_core::command::ExitCode;
use warp_core::{safe_info, safe_warn};
use warpui::r#async::{FutureExt, Timer};
use warpui::{ModelContext, ModelSpawner, SingletonEntity};

#[cfg(feature = "local_fs")]
use super::cache_setup;
use super::terminal::TerminalDriver;
use super::{AgentDriverError, Harness, failure_output, git_credentials};
use crate::ai::agent_sdk::environment_snapshot::{
    EnvironmentSnapshot, EnvironmentSnapshotReporter, RepositoryRevision,
};
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::cloud_environments::SourceRepo;
use crate::server::telemetry::secret_redaction::redact_secrets_in_string;
use crate::terminal::model::BlockId;
use crate::terminal::model::session::command_executor::shell_escape_single_quotes;
use crate::terminal::shell::ShellType;

const CODEBASE_INDEX_SYNC_TIMEOUT: Duration = Duration::from_secs(60);
const ENVIRONMENT_SNAPSHOT_CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);
const CLONE_FAILURE_IDENTITY_QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const CLONE_FAILURE_OUTPUT_TRUNCATION_MARKER: &str = "\n… clone output truncated …\n";
const SETUP_COMMAND_OUTPUT_TRUNCATION_MARKER: &str = "\n… setup command output truncated …\n";
const SETUP_COMMAND_TIMEOUT: Duration = Duration::from_mins(30);
const SETUP_COMMAND_CWD_RESET_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
pub enum SetupCommandPhase {
    Execute,
    ResetWorkingDirectory,
}

impl SetupCommandPhase {
    fn timeout(self) -> Duration {
        match self {
            Self::Execute => SETUP_COMMAND_TIMEOUT,
            Self::ResetWorkingDirectory => SETUP_COMMAND_CWD_RESET_TIMEOUT,
        }
    }
}

impl fmt::Display for SetupCommandPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Execute => "waiting for command to complete",
            Self::ResetWorkingDirectory => "resetting the working directory after the command",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrepareEnvironmentError {
    #[error("Invalid runtime state - please file a bug report.")]
    InvalidRuntimeState,
    #[error(
        "Failed to clone {repo_name}{}{identity_diagnostics}",
        clone_failure_output_suffix(.output.as_deref())
    )]
    CloneRepo {
        repo_name: String,
        output: Option<String>,
        identity_diagnostics: CloneFailureIdentityDiagnostics,
    },
    #[error("Failed to check out {checkout_ref} in {repo_name}{identity_diagnostics}")]
    CheckoutFailed {
        repo_name: String,
        checkout_ref: String,
        identity_diagnostics: CloneFailureIdentityDiagnostics,
    },
    #[error("Invalid repository preparation overrides: {reason}")]
    InvalidRepositoryPreparationOverrides { reason: String },
    #[error("Failed to remove origins from environment repositories")]
    RemoveRepositoryOrigins,
    #[error(
        "Failed to run setup command: {command}{}",
        setup_command_output_suffix(.output.as_deref())
    )]
    SetupCommand {
        command: String,
        output: Option<String>,
    },
    #[error(
        "Setup command #{command_index} timed out after {timeout_seconds}s while {phase}: `{command}`"
    )]
    SetupCommandTimedOut {
        command_index: usize,
        command: String,
        phase: SetupCommandPhase,
        timeout_seconds: u64,
    },
    #[error("Failed to change directory into {repo_name}")]
    ChangeDirectory { repo_name: String },
    #[error(
        "Repositories {first_owner}/{repo_name} and {second_owner}/{repo_name} share a clone directory name"
    )]
    CloneDirectoryCollision {
        repo_name: String,
        first_owner: String,
        second_owner: String,
    },
    #[error(
        "Repository {repo_name} has a code forge this client build doesn't support; update Warp to a version that does"
    )]
    UnsupportedRepositoryForge { repo_name: String },
    #[error("Environment checkout helper failed: {reason}")]
    CheckoutHelper { reason: &'static str },
    #[error("Terminal driver error while preparing environment: {source}")]
    TerminalDriver { source: AgentDriverError },
}

fn setup_command_output_suffix(output: Option<&str>) -> String {
    output
        .filter(|output| !output.is_empty())
        .map(|output| format!("\nCommand output:\n{output}"))
        .unwrap_or_default()
}

fn setup_command_failure(command: String, output: Option<String>) -> PrepareEnvironmentError {
    let output = output
        .map(|output| {
            failure_output::prepare_failure_output(&output, SETUP_COMMAND_OUTPUT_TRUNCATION_MARKER)
        })
        .filter(|output| !output.is_empty());
    PrepareEnvironmentError::SetupCommand { command, output }
}

async fn await_setup_phase<T>(
    command_index: usize,
    command: &str,
    phase: SetupCommandPhase,
    operation: impl Future<Output = Result<T, PrepareEnvironmentError>>,
    deadline: impl Future,
) -> Result<T, PrepareEnvironmentError> {
    let started_at = Instant::now();
    log::info!(
        "Environment setup lifecycle: event=phase_started command_index={command_index} phase={phase:?} timeout_seconds={}",
        phase.timeout().as_secs()
    );
    match select(Box::pin(operation), Box::pin(deadline)).await {
        Either::Left((result, _)) => {
            log::info!(
                "Environment setup lifecycle: event=phase_finished command_index={command_index} phase={phase:?} elapsed_ms={} result_ok={}",
                started_at.elapsed().as_millis(),
                result.is_ok()
            );
            result
        }
        Either::Right((_, _)) => {
            log::warn!(
                "Environment setup lifecycle: event=phase_timed_out command_index={command_index} phase={phase:?} elapsed_ms={}",
                started_at.elapsed().as_millis()
            );
            let mut command = command.to_owned();
            redact_secrets_in_string(&mut command);
            Err(PrepareEnvironmentError::SetupCommandTimedOut {
                command_index,
                command,
                phase,
                timeout_seconds: phase.timeout().as_secs(),
            })
        }
    }
}

fn clone_failure_output_suffix(output: Option<&str>) -> String {
    output
        .filter(|output| !output.is_empty())
        .map(|output| format!(": {output}"))
        .unwrap_or_default()
}

fn parse_resolved_head_sha(line: &str) -> Option<String> {
    let sha = line.trim();
    is_valid_git_object_id(sha).then(|| sha.to_string())
}

fn parse_resolved_head_shas(stdout: &[u8], repo_count: usize) -> Vec<Option<String>> {
    let Ok(stdout) = std::str::from_utf8(stdout) else {
        return vec![None; repo_count];
    };
    let mut resolved_heads = stdout
        .lines()
        .take(repo_count)
        .map(parse_resolved_head_sha)
        .collect::<Vec<_>>();
    resolved_heads.resize(repo_count, None);
    resolved_heads
}

fn build_resolved_head_command(
    repos: &[RepositoryCloneRequest],
    working_dir: &Path,
    shell_type: ShellType,
) -> String {
    repos
        .iter()
        .map(|request| {
            let escaped = shell_escape_single_quotes(
                &working_dir.join(&request.checkout_name).to_string_lossy(),
                shell_type,
            );
            match shell_type {
                ShellType::Bash | ShellType::Zsh => format!(
                    "git -C '{escaped}' rev-parse --verify HEAD 2>/dev/null || printf '\\n'"
                ),
                ShellType::Fish => format!(
                    "git -C '{escaped}' rev-parse --verify HEAD 2>/dev/null; or printf '\\n'"
                ),
                ShellType::PowerShell => format!(
                    "$head = & git -C '{escaped}' rev-parse --verify HEAD 2>$null; \
                 if ($LASTEXITCODE -eq 0) {{ Write-Output $head }} else {{ Write-Output '' }}"
                ),
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn checkout_path(working_dir: &Path, repo_name: &str) -> String {
    working_dir
        .join(repo_name)
        .strip_prefix(working_dir)
        .unwrap_or_else(|_| Path::new(repo_name))
        .to_string_lossy()
        .into_owned()
}

fn environment_snapshot(
    repos: &[RepositoryCloneRequest],
    working_dir: &Path,
    resolved_heads: &[Option<String>],
) -> EnvironmentSnapshot {
    let repositories = repos
        .iter()
        .zip(resolved_heads)
        .filter_map(|(request, resolved_head_sha)| {
            Some(RepositoryRevision {
                code_forge: request.remote.code_forge?,
                repo_owner: request.remote.owner.clone(),
                repo_name: request.remote.repo.clone(),
                checkout_path: checkout_path(working_dir, &request.checkout_name),
                requested_checkout_ref: request
                    .checkout
                    .as_ref()
                    .map(RepositoryHeadRef::value)
                    .map(str::to_string),
                resolved_head_sha: resolved_head_sha.clone()?,
            })
        })
        .collect::<Vec<_>>();
    if repositories.len() < repos.len() {
        log::warn!(
            "Could not capture resolved HEAD for {}/{} structured repositories",
            repos.len() - repositories.len(),
            repos.len()
        );
    }
    EnvironmentSnapshot {
        captured_at: Utc::now(),
        repositories,
    }
}

fn is_valid_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// Server-owned repository settings for environment preparation.
#[derive(Default)]
pub(crate) struct RepositoryPreparationOptions {
    source_repos: Vec<SourceRepo>,
    setup_commands: Vec<String>,
    preparation_overrides: Vec<RepositoryPreparationOverride>,
    remove_origins: bool,
}

impl RepositoryPreparationOptions {
    pub fn new(
        source_repos: Vec<SourceRepo>,
        setup_commands: Vec<String>,
        preparation_overrides: Vec<RepositoryPreparationOverride>,
        remove_origins: bool,
    ) -> Self {
        Self {
            source_repos,
            setup_commands,
            preparation_overrides,
            remove_origins,
        }
    }
}

pub(crate) fn validate_repository_preparation_overrides(
    source_repos: &[SourceRepo],
    overrides: &[RepositoryPreparationOverride],
) -> Result<(), PrepareEnvironmentError> {
    if source_repos.is_empty() && !overrides.is_empty() {
        return Err(
            PrepareEnvironmentError::InvalidRepositoryPreparationOverrides {
                reason: "repository preparation overrides require at least one repository"
                    .to_string(),
            },
        );
    }

    let mut source_identities = HashSet::new();
    for repo in source_repos {
        source_identities.insert(source_repo_identity(repo)?);
    }

    let mut override_identities = HashSet::new();
    for preparation_override in overrides {
        let identity = preparation_override.identity();
        if !override_identities.insert(identity.clone()) {
            return Err(
                PrepareEnvironmentError::InvalidRepositoryPreparationOverrides {
                    reason: format!(
                        "duplicate repository identity {:?}/{}/{}",
                        preparation_override.code_forge,
                        preparation_override.repo_owner,
                        preparation_override.repo_name
                    ),
                },
            );
        }
        if !source_identities.contains(&identity) {
            return Err(
                PrepareEnvironmentError::InvalidRepositoryPreparationOverrides {
                    reason: format!(
                        "repository {:?}/{}/{} is not declared by the environment",
                        preparation_override.code_forge,
                        preparation_override.repo_owner,
                        preparation_override.repo_name
                    ),
                },
            );
        }
    }

    Ok(())
}

/// Prepare a cloud agent environment within a terminal session. This will:
/// 1. Materialize all repositories, enforcing server-provided HEAD overrides.
/// 2. Begin codebase indexing for all repositories (Oz harness only).
/// 3. Run any setup commands.
/// 4. If there is only one repository, navigate into it.
///
/// Returns the directory where the harness will run.
///
/// `is_sandbox` tells the preparer that `working_dir` only exists inside a
/// Docker sandbox container and therefore the host filesystem can't be used
/// for repo detection or indexing. This is an explicit signal from the
/// caller rather than a path-prefix inference, so non-sandbox callers that
/// happen to pass a path like `/home/agent/...` don't silently flip into
/// sandbox-only mode.
pub(crate) fn prepare_environment(
    working_dir: PathBuf,
    is_sandbox: bool,
    harness: Harness,
    repository_options: RepositoryPreparationOptions,
    setup_events: SetupClientEventReporter,
    environment_snapshot_reporter: EnvironmentSnapshotReporter,
    ctx: &mut ModelContext<TerminalDriver>,
) -> impl Future<Output = Result<PathBuf, PrepareEnvironmentError>> + use<> {
    let spawner = ctx.spawner();
    async move {
        let RepositoryPreparationOptions {
            source_repos,
            setup_commands,
            preparation_overrides: repository_preparation_overrides,
            remove_origins: remove_repository_origins,
        } = repository_options;
        validate_repository_preparation_overrides(
            &source_repos,
            &repository_preparation_overrides,
        )?;
        // Only index the codebase for the Oz harness; third-party harnesses (e.g. Claude)
        // have their own methods for navigating a codebase.
        let should_index_codebase = harness == Harness::Oz;
        let should_subscribe_to_index_updates = should_index_codebase && !source_repos.is_empty();
        let repo_channels = Arc::new(Mutex::new(HashMap::<PathBuf, oneshot::Sender<()>>::new()));

        if should_subscribe_to_index_updates {
            subscribe_to_codebase_index_events(&spawner, Arc::clone(&repo_channels)).await?;
        }

        let result = prepare_environment_impl(
            &spawner,
            working_dir.as_path(),
            is_sandbox,
            &source_repos,
            &repository_preparation_overrides,
            remove_repository_origins,
            setup_commands,
            should_index_codebase,
            Arc::clone(&repo_channels),
            setup_events,
            environment_snapshot_reporter,
        )
        .await;

        if should_subscribe_to_index_updates && result.is_err() {
            let _ = spawner
                .spawn(|_, ctx| {
                    ctx.unsubscribe_from_model(&CodebaseIndexManager::handle(ctx));
                })
                .await;
        }

        result
    }
}

/// Merge environment repositories with task-level repositories, preserving
/// environment order and de-duplicating by forge plus case-insensitive owner
/// and repository names.
pub(crate) fn merge_repos_deduped(
    environment_repos: Vec<SourceRepo>,
    additional_repos: Vec<SourceRepo>,
) -> Result<Vec<SourceRepo>, PrepareEnvironmentError> {
    let mut seen = HashSet::new();
    let mut names = HashMap::<String, (String, Option<CodeForge>)>::new();
    let mut merged = Vec::with_capacity(environment_repos.len() + additional_repos.len());

    for repo in environment_repos.into_iter().chain(additional_repos) {
        let forge = repo.code_forge;
        let key = (forge, repo.owner.to_lowercase(), repo.repo.to_lowercase());
        if !seen.insert(key) {
            continue;
        }

        if let Some((owner, existing_forge)) =
            names.insert(repo.repo.to_lowercase(), (repo.owner.clone(), forge))
            && (!owner.eq_ignore_ascii_case(&repo.owner) || existing_forge != forge)
        {
            return Err(PrepareEnvironmentError::CloneDirectoryCollision {
                repo_name: repo.repo,
                first_owner: owner,
                second_owner: repo.owner,
            });
        }

        merged.push(repo);
    }

    Ok(merged)
}

/// Environment variable carrying the authenticated remote URL of a Factory's
/// definition repository. Dispatch attaches it only to runs that execute as a
/// Factory agent whose Factory definition lives in a Warp-managed repository.
const FACTORY_REPO_CLONE_URL_ENV_VAR: &str = "WARP_FACTORY_REPO_CLONE_URL";

/// Environment variable carrying the directory, relative to the working
/// directory, that the Factory definition repository is cloned into.
const FACTORY_REPO_DIR_ENV_VAR: &str = "WARP_FACTORY_REPO_DIR";

/// Prepends the setup command that clones a Factory's definition repository
/// when the dispatch attached the clone variables to this run, so the checkout
/// exists before user-declared setup commands run.
pub(super) fn prepend_factory_definition_clone(
    setup_commands: &mut Vec<String>,
    shell_type: Option<ShellType>,
) {
    let clone_url = std::env::var(FACTORY_REPO_CLONE_URL_ENV_VAR).unwrap_or_default();
    let clone_dir = std::env::var(FACTORY_REPO_DIR_ENV_VAR).unwrap_or_default();
    prepend_factory_definition_clone_for_values(
        &clone_url,
        &clone_dir,
        shell_type.unwrap_or(ShellType::Bash),
        setup_commands,
    );
}

fn prepend_factory_definition_clone_for_values(
    clone_url: &str,
    clone_dir: &str,
    shell_type: ShellType,
    setup_commands: &mut Vec<String>,
) {
    if clone_url.trim().is_empty() || clone_dir.trim().is_empty() {
        return;
    }
    // Environments provisioned before run-scoped cloning still persist their
    // own copy of the clone command; leave that copy in charge rather than
    // attempting the checkout twice.
    if setup_commands
        .iter()
        .any(|command| command.contains(FACTORY_REPO_CLONE_URL_ENV_VAR))
    {
        return;
    }
    // The command expands the variables in the session shell instead of
    // inlining their values so the credential-bearing URL never appears in
    // command text. There is deliberately no existence guard: a bare clone
    // into an already-present target directory fails, which is treated as a
    // fatal setup-command error upstream.
    setup_commands.insert(0, factory_definition_clone_command(shell_type));
}

fn factory_definition_clone_command(shell_type: ShellType) -> String {
    // PowerShell reads environment variables through the `env:` drive; a bare `$NAME` there is
    // an unset PowerShell variable that expands to an empty string.
    let env_var_reference = |name: &str| match shell_type {
        ShellType::PowerShell => format!("$env:{name}"),
        ShellType::Zsh | ShellType::Bash | ShellType::Fish => format!("${name}"),
    };
    format!(
        "git clone \"{}\" \"{}\"",
        env_var_reference(FACTORY_REPO_CLONE_URL_ENV_VAR),
        env_var_reference(FACTORY_REPO_DIR_ENV_VAR)
    )
}

#[allow(clippy::too_many_arguments)]
async fn prepare_environment_impl(
    spawner: &ModelSpawner<TerminalDriver>,
    working_dir: &Path,
    is_sandbox: bool,
    source_repos: &[SourceRepo],
    repository_preparation_overrides: &[RepositoryPreparationOverride],
    remove_repository_origins: bool,
    setup_commands: Vec<String>,
    should_index_codebase: bool,
    repo_channels: Arc<Mutex<HashMap<PathBuf, oneshot::Sender<()>>>>,
    setup_events: SetupClientEventReporter,
    environment_snapshot_reporter: EnvironmentSnapshotReporter,
) -> Result<PathBuf, PrepareEnvironmentError> {
    let working_dir_string = working_dir.to_string_lossy().to_string();

    // Position the session in `working_dir` before running any probes / clones.
    // Routed through the silent executor so we don't add a user-visible `cd`
    // block to the blocklist — in the common case (cloud agents) the session
    // is already cd'd here by its startup dir, so this is a no-op re-cd and
    // shouldn't appear in the user's terminal history.
    if !cd_in_terminal_silent(working_dir_string.clone(), spawner).await? {
        return Err(PrepareEnvironmentError::ChangeDirectory {
            repo_name: working_dir_string,
        });
    }
    let mut codebase_context_receivers = Vec::new();
    let repository_clone_requests = repository_clone_requests(
        source_repos,
        repository_preparation_overrides,
        remove_repository_origins,
    )?;

    // Snapshot the process-wide identity bootstrap set, before anything below
    // (cloning, setup commands) has a chance to change it for a given repo.
    // The post-setup-commands fallback below compares each repo's effective
    // identity against this baseline to tell whether the customer already
    // claimed that repo's identity, rather than assuming so from forge count.
    let git_identity_baseline = git_credentials::global_git_identity();

    let environment_snapshot = if repository_clone_requests.is_empty() {
        EnvironmentSnapshot::empty()
    } else {
        setup_events
            .record_result(SetupStep::EnvironmentRepoClone, async {
                clone_checkout_requests(&repository_clone_requests, working_dir, spawner).await
            })
            .await?
    };
    environment_snapshot_reporter.report(environment_snapshot);
    if !repository_clone_requests.is_empty() {
        for request in &repository_clone_requests {
            register_cloned_repo(&request.checkout_name, working_dir, is_sandbox, spawner).await?;
            if !is_sandbox && should_index_codebase {
                let receiver = index_repo_codebase(
                    &request.checkout_name,
                    working_dir,
                    Arc::clone(&repo_channels),
                    spawner,
                )
                .await?;
                if let Some(receiver) = receiver {
                    codebase_context_receivers.push(receiver);
                }
            }
        }

        if should_index_codebase {
            record_codebase_indexing(
                setup_events.clone(),
                spawner.clone(),
                codebase_context_receivers,
            );
        }
    }

    #[cfg(feature = "local_fs")]
    if let Some(cache_root) = cache_setup::enabled_cache_root() {
        log::info!("Configuring build cache");
        let result = setup_events
            .record_result(
                SetupStep::CacheSetup,
                cache_setup::setup_caches(
                    cache_root.clone(),
                    &repository_clone_requests,
                    super::environment_checkout::mirror_cache_usage(&cache_root),
                    working_dir,
                    spawner,
                ),
            )
            .await;
        if let Err(error) = result {
            log::warn!("Build cache setup degraded; continuing environment preparation: {error}");
        }
    } else {
        log::info!("Build cache not available");
    }

    let has_setup_commands = !setup_commands.is_empty();
    let setup_result = if has_setup_commands {
        setup_events
            .record_result(SetupStep::EnvironmentSetupCommands, async {
                // Set CI=true so setup commands run in a CI-like environment. This should help us run
                // non-interactive versions of setup commands, as many command line tools recognize the CI
                // environment variable.
                execute_command("export CI=true".to_string(), spawner).await?;

                for (index, command) in setup_commands.into_iter().enumerate() {
                    let command_index = index + 1;
                    let command_for_error = command.clone();
                    safe_info!(
                        safe: ("Running setup command"),
                        full: ("Running setup command: {command}")
                    );

                    let command_result = await_setup_phase(
                        command_index,
                        &command_for_error,
                        SetupCommandPhase::Execute,
                        execute_command(command, spawner),
                        Timer::after(SETUP_COMMAND_TIMEOUT),
                    )
                    .await?;
                    if command_result.exit_code != 0.into() {
                        let output =
                            fetch_block_output_plaintext(&command_result.block_id, spawner).await;
                        return Err(setup_command_failure(command_for_error, output));
                    }

                    let working_dir_string = working_dir.to_string_lossy().to_string();
                    let reset_result = await_setup_phase(
                        command_index,
                        &command_for_error,
                        SetupCommandPhase::ResetWorkingDirectory,
                        cd_in_terminal(working_dir_string, spawner),
                        Timer::after(SETUP_COMMAND_CWD_RESET_TIMEOUT),
                    )
                    .await;
                    if matches!(
                        &reset_result,
                        Err(PrepareEnvironmentError::SetupCommandTimedOut { .. })
                    ) {
                        reset_result?;
                    } else if let Err(error) = reset_result {
                        log::warn!(
                            "Failed to reset working directory after setup command: {error}"
                        );
                    }

                    safe_info!(
                        safe: ("Successfully completed setup command"),
                        full: ("Successfully completed setup command: {command_for_error}")
                    );
                }

                // Unset CI after setup commands complete so the agent session
                // does not run with CI=true.
                execute_command("unset CI".to_string(), spawner).await?;
                Ok::<(), PrepareEnvironmentError>(())
            })
            .await
    } else if should_index_codebase && source_repos.is_empty() {
        let _ = spawner
            .spawn(|_, ctx| {
                ctx.unsubscribe_from_model(&CodebaseIndexManager::handle(ctx));
            })
            .await;
        Ok(())
    } else {
        Ok(())
    };

    // A timed-out command may still own the terminal; issuing cleanup commands can hang again.
    let setup_result = match setup_result {
        Err(error @ PrepareEnvironmentError::SetupCommandTimedOut { .. }) => return Err(error),
        result => result,
    };

    // Fill in a forge-appropriate identity for any repo whose effective
    // identity is still exactly what bootstrap set — i.e. nothing (a setup
    // command, or anything else run above) has claimed it yet. This runs
    // after setup commands specifically so a customer's own git identity
    // config always wins: repo-local config always beats `--global` config
    // regardless of write order, so applying Warp's own fallback any earlier
    // would permanently shadow a later customer override. Runs even if a
    // setup command failed, so whatever happens next (e.g. a failure-linger
    // session) still has a usable identity for every repo.
    for request in &repository_clone_requests {
        git_credentials::configure_repository_git_identity_if_unset(
            &working_dir.join(&request.checkout_name),
            request.remote.code_forge.map(CodeForge::host).unwrap_or(""),
            git_identity_baseline.clone(),
        );
    }
    let remove_origins_result =
        remove_repository_origins_from_repos(&repository_clone_requests, working_dir, spawner)
            .await;
    setup_result?;
    remove_origins_result?;

    if should_index_codebase && source_repos.is_empty() {
        log::info!("No repositories to index for codebase context");
    }

    // If there's only one repo in the environment, start the agent in that repo.
    // This way, it doesn't have to locate the correct repo to work on.
    let harness_working_dir = if let Some(repo_name) = single_repo_name(source_repos) {
        safe_info!(
            safe: ("Changing directory into single repository"),
            full: ("Changing directory into single repository: {repo_name}")
        );
        let exit_code = cd_in_terminal(repo_name.clone(), spawner).await?;
        if exit_code != 0.into() {
            return Err(PrepareEnvironmentError::ChangeDirectory { repo_name });
        }
        working_dir.join(repo_name)
    } else {
        working_dir.to_path_buf()
    };
    Ok(harness_working_dir)
}

fn record_codebase_indexing(
    setup_events: SetupClientEventReporter,
    spawner: ModelSpawner<TerminalDriver>,
    codebase_context_receivers: Vec<oneshot::Receiver<()>>,
) {
    if codebase_context_receivers.is_empty() {
        setup_events.record_value_detached(SetupStep::EnvironmentCodebaseIndexing, async move {
            let _ = spawner
                .spawn(|_, ctx| {
                    ctx.unsubscribe_from_model(&CodebaseIndexManager::handle(ctx));
                })
                .await;
        });
        return;
    }

    setup_events.record_value_detached(SetupStep::EnvironmentCodebaseIndexing, async move {
        let repos_indexed = join_all(codebase_context_receivers);
        if repos_indexed
            .with_timeout(CODEBASE_INDEX_SYNC_TIMEOUT)
            .await
            .is_err()
        {
            log::warn!(
                "Timed out waiting for codebase index sync; continuing without guaranteed codebase context",
            );
            tracing::warn!(
                "Timed out waiting for codebase index sync; continuing without guaranteed codebase context",
            );
        }
        let _ = spawner
            .spawn(|_, ctx| {
                ctx.unsubscribe_from_model(&CodebaseIndexManager::handle(ctx));
            })
            .await;
    });
}

// `None` covers both a repo-less container forge and one this client build
// doesn't recognize. Unlike `None`, a future server can assign the latter to
// a real repository before this client updates, so callers must treat it as
// an ordinary "can't clone this" outcome rather than an invariant violation.
fn repository_forge_for_repo(repo: &SourceRepo) -> Option<RepositoryForge> {
    match repo.code_forge {
        Some(CodeForge::GitHub) => Some(RepositoryForge::GitHub),
        Some(CodeForge::GitLab) => Some(RepositoryForge::GitLab),
        Some(CodeForge::AzureDevOps) => Some(RepositoryForge::AzureDevOps),
        Some(CodeForge::None | CodeForge::Unknown) | None => None,
    }
}

fn code_forge_for_repository_forge(forge: RepositoryForge) -> CodeForge {
    match forge {
        RepositoryForge::GitHub => CodeForge::GitHub,
        RepositoryForge::GitLab => CodeForge::GitLab,
        RepositoryForge::AzureDevOps => CodeForge::AzureDevOps,
    }
}

fn source_repo_identity(
    repo: &SourceRepo,
) -> Result<(RepositoryForge, String, String), PrepareEnvironmentError> {
    let Some(forge) = repository_forge_for_repo(repo) else {
        return Err(PrepareEnvironmentError::UnsupportedRepositoryForge {
            repo_name: format!("{}/{}", repo.owner, repo.repo),
        });
    };
    if repo.owner.is_empty()
        || repo.owner.trim() != repo.owner
        || repo.repo.is_empty()
        || repo.repo.trim() != repo.repo
    {
        return Err(
            PrepareEnvironmentError::InvalidRepositoryPreparationOverrides {
                reason: format!(
                    "repository identity {:?}/{}/{} must be non-empty without surrounding whitespace",
                    forge, repo.owner, repo.repo
                ),
            },
        );
    }
    Ok((forge, repo.owner.to_lowercase(), repo.repo.to_lowercase()))
}

fn preparation_override_matches_repo(
    preparation_override: &RepositoryPreparationOverride,
    repo: &SourceRepo,
) -> bool {
    source_repo_identity(repo).is_ok_and(|identity| identity == preparation_override.identity())
}

fn preparation_override_for_repo<'a>(
    overrides: &'a [RepositoryPreparationOverride],
    repo: &SourceRepo,
) -> Option<&'a RepositoryPreparationOverride> {
    overrides
        .iter()
        .find(|preparation_override| preparation_override_matches_repo(preparation_override, repo))
}

#[derive(Debug, Clone)]
pub(super) struct RepositoryCloneRequest {
    pub(super) remote: SourceRepo,
    pub(super) checkout_name: String,
    pub(super) checkout: Option<RepositoryHeadRef>,
    pub(super) remove_origin: bool,
    pub(super) fetch_branch_only: bool,
}

fn unique_clone_hosts<'a>(
    requests: impl IntoIterator<Item = &'a RepositoryCloneRequest>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    requests
        .into_iter()
        .filter_map(|request| request.remote.code_forge.map(CodeForge::host))
        .filter(|host| seen.insert(*host))
        .map(str::to_string)
        .collect()
}

fn sanitize_git_author_name(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_alphanumeric()
                || matches!(character, ' ' | '.' | '_' | '@' | '+' | '-' | '\'')
        }))
    .then(|| value.to_string())
}

fn sanitize_git_credential_username(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_alphanumeric() || matches!(character, '.' | '_' | '@' | '+' | '-')
        }))
    .then(|| value.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CloneFailureCredentialIdentity {
    host: String,
    username: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneFailureIdentityDiagnostics {
    author: Option<String>,
    credentials: Vec<CloneFailureCredentialIdentity>,
}

impl fmt::Display for CloneFailureIdentityDiagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let author = self.author.as_deref().unwrap_or("unset");
        write!(formatter, "\nGit identity diagnostics:\n  Author: {author}")?;
        for credential in &self.credentials {
            let username = credential.username.as_deref().unwrap_or("unavailable");
            write!(
                formatter,
                "\n  Credential username for {}: {username}",
                credential.host
            )?;
        }
        Ok(())
    }
}

fn git_credential_username(output: &CommandOutput) -> Option<String> {
    if !output.success() {
        return None;
    }
    let stdout = std::str::from_utf8(&output.stdout).ok()?;
    let mut usernames = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("username="));
    let username = usernames.next()?;
    if usernames.next().is_some() {
        return None;
    }
    sanitize_git_credential_username(username)
}

fn clone_failure_identity_diagnostics<'a>(
    author_output: Option<&CommandOutput>,
    credential_outputs: impl IntoIterator<Item = (&'a str, Option<&'a CommandOutput>)>,
) -> CloneFailureIdentityDiagnostics {
    let author = author_output
        .filter(|output| output.success())
        .and_then(|output| std::str::from_utf8(&output.stdout).ok())
        .and_then(sanitize_git_author_name);
    let credentials = credential_outputs
        .into_iter()
        .map(|(host, output)| CloneFailureCredentialIdentity {
            host: host.to_string(),
            username: output.and_then(git_credential_username),
        })
        .collect();
    CloneFailureIdentityDiagnostics {
        author,
        credentials,
    }
}

fn build_git_credential_query_command(host: &str, shell_type: ShellType) -> String {
    let credential_input = format!("protocol=https\nhost={host}\n");
    let escaped_input = shell_escape_single_quotes(&credential_input, shell_type);
    match shell_type {
        ShellType::Bash | ShellType::Zsh | ShellType::Fish => format!(
            "printf '%s\\n' '{escaped_input}' | env GIT_TERMINAL_PROMPT=0 \
             GCM_INTERACTIVE=never git credential fill"
        ),
        ShellType::PowerShell => format!(
            "$env:GIT_TERMINAL_PROMPT='0'; $env:GCM_INTERACTIVE='never'; \
             '{escaped_input}' | & git credential fill"
        ),
    }
}

async fn collect_clone_failure_identity_diagnostics(
    hosts: Vec<String>,
    spawner: &ModelSpawner<TerminalDriver>,
) -> CloneFailureIdentityDiagnostics {
    let shell_type = active_shell_type(spawner).await;
    let author_query = execute_silent_command("git config --get user.name".to_string(), spawner)
        .with_timeout(CLONE_FAILURE_IDENTITY_QUERY_TIMEOUT);
    let credential_queries = hosts.into_iter().map(|host| async move {
        let command = build_git_credential_query_command(&host, shell_type);
        let output = execute_silent_command(command, spawner)
            .with_timeout(CLONE_FAILURE_IDENTITY_QUERY_TIMEOUT)
            .await;
        let output = match output {
            Ok(Ok(output)) if output.success() => Some(output),
            Ok(Ok(_)) | Ok(Err(_)) | Err(_) => None,
        };
        (host, output)
    });
    let (author_output, credential_outputs) =
        futures::join!(author_query, join_all(credential_queries));
    let author_output = match author_output {
        Ok(Ok(output)) if output.success() => Some(output),
        Ok(Ok(_)) | Ok(Err(_)) | Err(_) => None,
    };
    clone_failure_identity_diagnostics(
        author_output.as_ref(),
        credential_outputs
            .iter()
            .map(|(host, output)| (host.as_str(), output.as_ref())),
    )
}

fn repository_clone_requests(
    repos: &[SourceRepo],
    overrides: &[RepositoryPreparationOverride],
    remove_repository_origins: bool,
) -> Result<Vec<RepositoryCloneRequest>, PrepareEnvironmentError> {
    validate_repository_preparation_overrides(repos, overrides)?;
    repos
        .iter()
        .cloned()
        .map(|repo| {
            source_repo_identity(&repo)?;
            let preparation_override = preparation_override_for_repo(overrides, &repo);
            let checkout = preparation_override
                .map(|preparation_override| preparation_override.head.clone())
                .or_else(|| repo.checkout_ref.clone().map(RepositoryHeadRef::Branch));
            let remote = match preparation_override
                .and_then(|preparation_override| preparation_override.clone_from.as_ref())
            {
                Some(identity) => SourceRepo::new(
                    code_forge_for_repository_forge(identity.code_forge),
                    identity.repo_owner.clone(),
                    identity.repo_name.clone(),
                ),
                None => repo.clone(),
            };
            let remove_origin = remove_repository_origins
                && !preparation_override
                    .is_some_and(|preparation_override| preparation_override.preserve_origin);
            Ok(RepositoryCloneRequest {
                remote,
                checkout_name: repo.repo,
                checkout,
                remove_origin,
                fetch_branch_only: preparation_override.is_some_and(|preparation_override| {
                    preparation_override.clone_from.is_some()
                        && matches!(preparation_override.head, RepositoryHeadRef::Branch(_))
                }),
            })
        })
        .collect()
}

async fn active_shell_type(spawner: &ModelSpawner<TerminalDriver>) -> ShellType {
    spawner
        .spawn(|driver, ctx| {
            driver
                .active_session_shell_type(ctx)
                .unwrap_or(ShellType::Bash)
        })
        .await
        .unwrap_or(ShellType::Bash)
}

async fn remove_repository_origins_from_repos(
    repos: &[RepositoryCloneRequest],
    working_dir: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(), PrepareEnvironmentError> {
    if !repos.iter().any(|request| request.remove_origin) {
        return Ok(());
    }
    let selected = repos
        .iter()
        .filter(|request| request.remove_origin)
        .cloned()
        .collect::<Vec<_>>();
    let batch = checkout_requests_batch(&selected, working_dir)?;
    let (output, _) = execute_checkout_helper(&batch, true, spawner)
        .await
        .map_err(|_| PrepareEnvironmentError::RemoveRepositoryOrigins)?;
    if output.exit_code == 0.into() {
        Ok(())
    } else {
        Err(PrepareEnvironmentError::RemoveRepositoryOrigins)
    }
}

/// Clone all source repositories to `{working_dir}/{repo.repo}` if they do not already exist.
/// Multiple repositories are cloned in parallel to reduce environment setup time.
pub(super) async fn clone_repos(
    repos: &[SourceRepo],
    working_dir: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(), PrepareEnvironmentError> {
    clone_checkout_requests(
        &repository_clone_requests(repos, &[], false)?,
        working_dir,
        spawner,
    )
    .await
    .map(|_| ())
}

fn build_checkout_helper_command(
    executable: &Path,
    requests_file: &Path,
    failure_report: &Path,
    remove_origins_only: bool,
    shell_type: ShellType,
) -> String {
    let quote = |path: &Path| {
        format!(
            "'{}'",
            shell_escape_single_quotes(&path.to_string_lossy(), shell_type)
        )
    };
    let prefix = if shell_type == ShellType::PowerShell {
        "& "
    } else {
        ""
    };
    let operation = if remove_origins_only {
        " --remove-origins-only"
    } else {
        ""
    };
    format!(
        "{prefix}{} environment-checkout --requests-file {} --failure-report {}{operation}",
        quote(executable),
        quote(requests_file),
        quote(failure_report)
    )
}

fn read_checkout_failures(path: &Path, request_count: usize) -> Option<Vec<CheckoutFailure>> {
    let bytes = std::fs::read(path);
    let _ = std::fs::remove_file(path);
    let report: CheckoutFailureReport = serde_json::from_slice(&bytes.ok()?).ok()?;
    let mut indexes = HashSet::new();
    (!report.failures.is_empty()
        && report.failures.iter().all(|failure| {
            failure.request_index < request_count && indexes.insert(failure.request_index)
        }))
    .then_some(report.failures)
}

fn checkout_requests_batch(
    repos: &[RepositoryCloneRequest],
    working_dir: &Path,
) -> Result<CheckoutBatch, PrepareEnvironmentError> {
    let batch = CheckoutBatch {
        working_dir: working_dir.to_owned(),
        repositories: repos
            .iter()
            .map(|request| {
                let code_forge = repository_forge_for_repo(&request.remote).ok_or_else(|| {
                    PrepareEnvironmentError::UnsupportedRepositoryForge {
                        repo_name: request.remote.to_string(),
                    }
                })?;
                Ok(CheckoutRequest {
                    source: RepositoryIdentity {
                        code_forge,
                        repo_owner: request.remote.owner.clone(),
                        repo_name: request.remote.repo.clone(),
                    },
                    checkout_name: request.checkout_name.clone(),
                    head: request.checkout.clone(),
                    fetch_branch_only: request.fetch_branch_only,
                })
            })
            .collect::<Result<Vec<_>, PrepareEnvironmentError>>()?,
    };
    batch.validate().map_err(|reason| {
        PrepareEnvironmentError::InvalidRepositoryPreparationOverrides {
            reason: reason.to_owned(),
        }
    })?;
    Ok(batch)
}

async fn execute_checkout_helper(
    batch: &CheckoutBatch,
    remove_origins_only: bool,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(ExecutedCommand, Option<Vec<CheckoutFailure>>), PrepareEnvironmentError> {
    let directory = tempfile::tempdir().map_err(|_| PrepareEnvironmentError::CheckoutHelper {
        reason: "could not create private checkout request directory",
    })?;
    let requests_file = directory.path().join("requests.json");
    let failure_report = directory.path().join("failures.json");
    let bytes = serde_json::to_vec(batch).map_err(|_| PrepareEnvironmentError::CheckoutHelper {
        reason: "could not serialize checkout requests",
    })?;
    std::fs::write(&requests_file, bytes).map_err(|_| PrepareEnvironmentError::CheckoutHelper {
        reason: "could not write checkout requests",
    })?;
    let executable =
        std::env::current_exe().map_err(|_| PrepareEnvironmentError::CheckoutHelper {
            reason: "could not resolve the running Warp/Oz executable",
        })?;
    let command = build_checkout_helper_command(
        &executable,
        &requests_file,
        &failure_report,
        remove_origins_only,
        active_shell_type(spawner).await,
    );
    let result = execute_command(command, spawner).await;
    let _ = std::fs::remove_file(&requests_file);
    let command_result = result?;
    let failures = read_checkout_failures(&failure_report, batch.repositories.len());
    Ok((command_result, failures))
}

async fn clone_checkout_requests(
    repos: &[RepositoryCloneRequest],
    working_dir: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<EnvironmentSnapshot, PrepareEnvironmentError> {
    if repos.is_empty() {
        return Ok(EnvironmentSnapshot::empty());
    }
    let batch = checkout_requests_batch(repos, working_dir)?;
    let (command_result, failures) = execute_checkout_helper(&batch, false, spawner).await?;
    if command_result.exit_code != 0.into() {
        let failed_requests = match &failures {
            Some(failures) => failures
                .iter()
                .map(|failure| &repos[failure.request_index])
                .collect(),
            None => repos.iter().collect::<Vec<_>>(),
        };
        let repo_name = failed_requests
            .iter()
            .map(|request| request.remote.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let identity_diagnostics = collect_clone_failure_identity_diagnostics(
            unique_clone_hosts(failed_requests.iter().copied()),
            spawner,
        )
        .await;
        if let Some(failures) = &failures
            && failures
                .iter()
                .all(|failure| failure.kind == CheckoutFailureKind::Checkout)
        {
            return Err(PrepareEnvironmentError::CheckoutFailed {
                repo_name,
                checkout_ref: failed_requests
                    .iter()
                    .filter_map(|request| request.checkout.as_ref())
                    .map(RepositoryHeadRef::value)
                    .collect::<Vec<_>>()
                    .join(", "),
                identity_diagnostics,
            });
        }
        let output = match failures {
            Some(failures) => Some(failure_output::prepare_failure_output(
                &failures
                    .iter()
                    .map(|failure| failure.output.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                CLONE_FAILURE_OUTPUT_TRUNCATION_MARKER,
            )),
            None => fetch_clone_failure_output(&command_result.block_id, spawner).await,
        };
        return Err(PrepareEnvironmentError::CloneRepo {
            repo_name,
            output,
            identity_diagnostics,
        });
    }
    Ok(capture_environment_snapshot(repos, working_dir, spawner).await)
}

async fn capture_environment_snapshot(
    repos: &[RepositoryCloneRequest],
    working_dir: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> EnvironmentSnapshot {
    if repos.is_empty() {
        return EnvironmentSnapshot::empty();
    }
    let command = build_resolved_head_command(repos, working_dir, active_shell_type(spawner).await);
    let resolved_heads = match execute_silent_command(command, spawner)
        .with_timeout(ENVIRONMENT_SNAPSHOT_CAPTURE_TIMEOUT)
        .await
    {
        Ok(Ok(output)) => parse_resolved_head_shas(&output.stdout, repos.len()),
        Ok(Err(error)) => {
            log::warn!("Could not capture resolved HEADs for structured repositories: {error}");
            vec![None; repos.len()]
        }
        Err(_) => {
            log::warn!(
                "Timed out capturing resolved HEADs for structured repositories after {:?}",
                ENVIRONMENT_SNAPSHOT_CAPTURE_TIMEOUT
            );
            vec![None; repos.len()]
        }
    };
    environment_snapshot(repos, working_dir, &resolved_heads)
}

/// Register a cloned source repository with `DetectedRepositories` so that the
/// skill watcher and other repo-aware subsystems can discover it.
#[tracing::instrument(skip_all, err, fields(tags.cloud_agent = true, repo = checkout_name, is_sandbox = is_sandbox))]
pub(super) async fn register_cloned_repo(
    checkout_name: &str,
    working_dir: &Path,
    is_sandbox: bool,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(), PrepareEnvironmentError> {
    let repo_dir = working_dir.join(checkout_name);

    // Register the repo with DetectedRepositories so that the skill watcher
    // and other repo-aware subsystems can discover it before the first query.
    //
    // TODO(advait): When the remote code server lands for Docker sandboxes,
    // sandbox-only working directories will be reachable from the host and
    // we should register + index them here too (likely via a remote-aware
    // path instead of `detect_possible_local_git_repo`/`index_directory`, which
    // both assume a local filesystem). For now, skip so we don't try to
    // stat paths that only exist inside the sandbox.
    if is_sandbox {
        safe_info!(
            safe: ("Skipping local repo detection for sandbox-only working directory"),
            full: (
                "Skipping local repo detection and indexing for sandbox-only working directory {}",
                working_dir.display()
            )
        );
    } else {
        let repo_dir_str = repo_dir.to_string_lossy().to_string();
        let detect_future = spawner
            .spawn(move |_, ctx| {
                DetectedRepositories::handle(ctx).update(ctx, |repos, ctx| {
                    repos.detect_possible_local_git_repo(
                        &repo_dir_str,
                        RepoDetectionSource::CloudEnvironmentPrep,
                        ctx,
                    )
                })
            })
            .await
            .map_err(|_| PrepareEnvironmentError::InvalidRuntimeState)?;
        // Await detection so the repo is registered in DirectoryWatcher
        // before the agent's first query.
        if detect_future.await.is_none() {
            safe_warn!(
                safe: ("Repository detection returned no path"),
                full: ("Repository detection returned no path for {}", repo_dir.display())
            );
        }
    }

    Ok(())
}

async fn subscribe_to_codebase_index_events(
    spawner: &ModelSpawner<TerminalDriver>,
    repo_channels: Arc<Mutex<HashMap<PathBuf, oneshot::Sender<()>>>>,
) -> Result<(), PrepareEnvironmentError> {
    spawner
        .spawn(move |_, ctx| {
            let repo_channels = Arc::clone(&repo_channels);
            ctx.subscribe_to_model(&CodebaseIndexManager::handle(ctx), move |_, _, event, ctx| {
                    if !matches!(
                        event,
                        CodebaseIndexManagerEvent::SyncStateUpdated { .. }
                    ) {
                        return;
                    }

                    let manager = CodebaseIndexManager::as_ref(ctx);
                    let mut repos_to_notify = Vec::new();
                    let mut channels = repo_channels
                        .lock()
                        .expect("repo channel map lock should not be poisoned");

                    for repo in channels.keys() {
                        let Some(status) =
                            manager.get_codebase_index_status_for_path(repo, ctx)
                        else {
                            continue;
                        };

                        if status.has_synced_version() {
                            repos_to_notify.push(repo.clone());
                            continue;
                        }

                        if !status.has_pending() && status.last_sync_successful() == Some(false) {
                            safe_warn!(
                                safe: ("Codebase index sync failed for a repo; unblocking environment setup"),
                                full: ("Codebase index sync failed for {repo:?}; unblocking environment setup")
                            );
                            repos_to_notify.push(repo.clone());
                        }
                    }

                    for repo in repos_to_notify {
                        if let Some(tx) = channels.remove(&repo) {
                            let _ = tx.send(());
                        }
                    }
                });
        })
        .await
        .map_err(|_| PrepareEnvironmentError::InvalidRuntimeState)
}

#[tracing::instrument(skip_all, err, fields(tags.cloud_agent = true, repo = %repo_name))]
async fn index_repo_codebase(
    repo_name: &str,
    working_dir: &Path,
    repo_channels: Arc<Mutex<HashMap<PathBuf, oneshot::Sender<()>>>>,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<Option<oneshot::Receiver<()>>, PrepareEnvironmentError> {
    let repo_path = working_dir.join(repo_name);

    safe_info!(
        safe: ("Trying to index repository for codebase context"),
        full: ("Trying to index {:?} for codebase context", repo_path)
    );

    let repo_path_for_spawn = repo_path.clone();
    spawner
        .spawn(move |_, ctx| {
            CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.index_directory(repo_path_for_spawn.clone(), ctx);
            });

            let status = CodebaseIndexManager::as_ref(ctx)
                .get_codebase_index_status_for_path(&repo_path_for_spawn, ctx);

            match status {
                Some(status) if status.has_synced_version() => {
                    safe_info!(
                        safe: ("Not waiting on codebase index for repository; we have one already"),
                        full: ("Not waiting on codebase index for {:?}, we have one already", repo_path_for_spawn)
                    );
                    None
                }
                _ => {
                    safe_info!(
                        safe: ("Waiting on codebase index for repository"),
                        full: ("Waiting on codebase index for {:?}", repo_path_for_spawn)
                    );
                    let (tx, rx) = oneshot::channel::<()>();
                    repo_channels
                        .lock()
                        .expect("repo channel map lock should not be poisoned")
                        .insert(repo_path_for_spawn, tx);
                    Some(rx)
                }
            }
        })
        .await
        .map_err(|_| PrepareEnvironmentError::InvalidRuntimeState)
}

struct ExecutedCommand {
    exit_code: ExitCode,
    block_id: BlockId,
}

/// Execute a command in the context of a terminal session.
async fn execute_command(
    command: String,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<ExecutedCommand, PrepareEnvironmentError> {
    let command_handle = spawner
        .spawn(move |terminal_driver, ctx| terminal_driver.execute_command(&command, ctx))
        .await
        .map_err(|_| PrepareEnvironmentError::InvalidRuntimeState)?
        .map_err(|error| match error {
            AgentDriverError::InvalidRuntimeState => PrepareEnvironmentError::InvalidRuntimeState,
            source => PrepareEnvironmentError::TerminalDriver { source },
        })?
        .await
        .map_err(|error| match error {
            AgentDriverError::InvalidRuntimeState => PrepareEnvironmentError::InvalidRuntimeState,
            source => PrepareEnvironmentError::TerminalDriver { source },
        })?;
    let block_id = command_handle.block_id().clone();
    let exit_code = command_handle.await.map_err(|error| match error {
        AgentDriverError::InvalidRuntimeState => PrepareEnvironmentError::InvalidRuntimeState,
        source => PrepareEnvironmentError::TerminalDriver { source },
    })?;
    Ok(ExecutedCommand {
        exit_code,
        block_id,
    })
}

async fn fetch_clone_failure_output(
    block_id: &BlockId,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Option<String> {
    fetch_block_output_plaintext(block_id, spawner)
        .await
        .map(|output| {
            failure_output::prepare_failure_output(&output, CLONE_FAILURE_OUTPUT_TRUNCATION_MARKER)
        })
        .filter(|output| !output.is_empty())
}

async fn fetch_block_output_plaintext(
    block_id: &BlockId,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Option<String> {
    let block_id = block_id.clone();
    spawner
        .spawn(move |driver, ctx| driver.block_output_plaintext(&block_id, ctx))
        .await
        .ok()
        .flatten()
}

async fn execute_silent_command(
    command: String,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<CommandOutput, PrepareEnvironmentError> {
    spawner
        .spawn(move |driver, ctx| driver.execute_silent_command(command, ctx))
        .await
        .map_err(|_| PrepareEnvironmentError::InvalidRuntimeState)?
        .await
        .map_err(|error| match error {
            AgentDriverError::InvalidRuntimeState => PrepareEnvironmentError::InvalidRuntimeState,
            source => PrepareEnvironmentError::TerminalDriver { source },
        })
}

/// Change the current directory in the context of a terminal session (using `cd {dir}`).
async fn cd_in_terminal(
    target: String,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<ExitCode, PrepareEnvironmentError> {
    spawner
        .spawn(move |terminal_driver, ctx| terminal_driver.cd(&target, ctx))
        .await
        .map_err(|_| PrepareEnvironmentError::InvalidRuntimeState)?
        .map_err(|error| match error {
            AgentDriverError::InvalidRuntimeState => PrepareEnvironmentError::InvalidRuntimeState,
            source => PrepareEnvironmentError::TerminalDriver { source },
        })?
        .await
        .map_err(|error| match error {
            AgentDriverError::InvalidRuntimeState => PrepareEnvironmentError::InvalidRuntimeState,
            source => PrepareEnvironmentError::TerminalDriver { source },
        })?
        .await
        .map_err(|error| match error {
            AgentDriverError::InvalidRuntimeState => PrepareEnvironmentError::InvalidRuntimeState,
            source => PrepareEnvironmentError::TerminalDriver { source },
        })
}

fn single_repo_name(repos: &[SourceRepo]) -> Option<String> {
    if repos.len() != 1 {
        return None;
    }
    Some(repos[0].repo.clone())
}

/// Change the active terminal session's working directory via `cd <target>`,
/// silently.
///
/// Thin wrapper around [`TerminalDriver::cd_silent`] so the call stays
/// consistent with the other `*_in_terminal` / `terminal_*` helpers in this
/// module. Uses the same [`ShellFamily::shell_escape`] logic as the visible
/// [`TerminalDriver::cd`] path, so it's safe across bash/zsh/fish/pwsh host
/// shells.
///
/// Returns `true` if the `cd` exited successfully.
async fn cd_in_terminal_silent(
    target: String,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<bool, PrepareEnvironmentError> {
    let output = spawner
        .spawn(move |driver, ctx| driver.cd_silent(&target, ctx))
        .await
        .map_err(|_| PrepareEnvironmentError::InvalidRuntimeState)?
        .await
        .map_err(|error| match error {
            AgentDriverError::InvalidRuntimeState => PrepareEnvironmentError::InvalidRuntimeState,
            source => PrepareEnvironmentError::TerminalDriver { source },
        })?;
    Ok(output.status == CommandExitStatus::Success)
}

#[cfg(test)]
#[path = "environment_tests.rs"]
mod tests;
