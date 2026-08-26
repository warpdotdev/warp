use std::collections::{HashMap, HashSet};
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
use futures::future::join_all;
use repo_metadata::repositories::{DetectedRepositories, RepoDetectionSource};
use warp_cli::agent::{Harness, RepositoryForge, RepositoryHeadOverride, RepositoryHeadRef};
use warp_completer::completer::{CommandExitStatus, CommandOutput};
use warp_core::command::ExitCode;
use warp_core::{safe_info, safe_warn};
use warpui::r#async::FutureExt;
use warpui::{ModelContext, ModelSpawner, SingletonEntity};

use super::AgentDriverError;
#[cfg(feature = "local_fs")]
use super::cache_setup;
use super::terminal::TerminalDriver;
use crate::ai::agent_sdk::environment_snapshot::{
    EnvironmentSnapshot, EnvironmentSnapshotReporter, RepositoryRevision,
};
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::cloud_environments::SourceRepo;
use crate::terminal::model::session::command_executor::shell_escape_single_quotes;
use crate::terminal::shell::ShellType;

const CODEBASE_INDEX_SYNC_TIMEOUT: Duration = Duration::from_secs(60);
const ENVIRONMENT_SNAPSHOT_CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum PrepareEnvironmentError {
    #[error("Invalid runtime state - please file a bug report.")]
    InvalidRuntimeState,
    #[error("Failed to clone {repo_name}")]
    CloneRepo { repo_name: String },
    #[error("Failed to check out {checkout_ref} in {repo_name}")]
    CheckoutFailed {
        repo_name: String,
        checkout_ref: String,
    },
    #[error("Invalid repository HEAD overrides: {reason}")]
    InvalidRepositoryHeadOverrides { reason: String },
    #[error("Failed to remove origins from environment repositories")]
    RemoveRepositoryOrigins,
    #[error("Failed to run setup command: {command}")]
    SetupCommand { command: String },
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
    #[error("Terminal driver error while preparing environment: {source}")]
    TerminalDriver { source: AgentDriverError },
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

fn build_resolved_head_command(repos: &[RepositoryCloneRequest], working_dir: &Path) -> String {
    let mut script = String::from("set +e\n");
    for request in repos {
        let escaped = shell_escape_single_quotes(
            &working_dir.join(&request.repo.repo).to_string_lossy(),
            ShellType::Bash,
        );
        script.push_str(&format!(
            "sha=\"$(git -C '{escaped}' rev-parse --verify HEAD 2>/dev/null)\"\n\
             printf '%s\\n' \"$sha\"\n"
        ));
    }
    format!(
        "sh -c '{}'",
        shell_escape_single_quotes(&script, ShellType::Bash)
    )
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
                code_forge: request.repo.code_forge.unwrap_or_default(),
                repo_owner: request.repo.owner.clone(),
                repo_name: request.repo.repo.clone(),
                checkout_path: checkout_path(working_dir, &request.repo.repo),
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
    head_overrides: Vec<RepositoryHeadOverride>,
    remove_origins: bool,
}

impl RepositoryPreparationOptions {
    pub fn new(
        source_repos: Vec<SourceRepo>,
        setup_commands: Vec<String>,
        head_overrides: Vec<RepositoryHeadOverride>,
        remove_origins: bool,
    ) -> Self {
        Self {
            source_repos,
            setup_commands,
            head_overrides,
            remove_origins,
        }
    }
}

pub(crate) fn validate_repository_head_overrides(
    source_repos: &[SourceRepo],
    overrides: &[RepositoryHeadOverride],
) -> Result<(), PrepareEnvironmentError> {
    if overrides.is_empty() {
        return Ok(());
    }
    if source_repos.is_empty() {
        return Err(PrepareEnvironmentError::InvalidRepositoryHeadOverrides {
            reason: "repository HEAD overrides require at least one repository".to_string(),
        });
    }
    let mut identities = HashSet::new();
    for head_override in overrides {
        if !identities.insert(head_override.identity()) {
            return Err(PrepareEnvironmentError::InvalidRepositoryHeadOverrides {
                reason: format!(
                    "duplicate repository identity {:?}/{}/{}",
                    head_override.code_forge, head_override.repo_owner, head_override.repo_name
                ),
            });
        }
        if !source_repos
            .iter()
            .any(|repo| head_override_matches_repo(head_override, repo))
        {
            return Err(PrepareEnvironmentError::InvalidRepositoryHeadOverrides {
                reason: format!(
                    "repository {:?}/{}/{} is not declared by the environment",
                    head_override.code_forge, head_override.repo_owner, head_override.repo_name
                ),
            });
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
) -> impl Future<Output = Result<(), PrepareEnvironmentError>> + use<> {
    let spawner = ctx.spawner();
    async move {
        let RepositoryPreparationOptions {
            source_repos,
            setup_commands,
            head_overrides: repository_head_overrides,
            remove_origins: remove_repository_origins,
        } = repository_options;
        validate_repository_head_overrides(&source_repos, &repository_head_overrides)?;
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
            &repository_head_overrides,
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
    let mut names = HashMap::<String, (String, CodeForge)>::new();
    let mut merged = Vec::with_capacity(environment_repos.len() + additional_repos.len());

    for repo in environment_repos.into_iter().chain(additional_repos) {
        let forge = repo.code_forge.unwrap_or_default();
        let key = (forge, repo.owner.to_lowercase(), repo.repo.to_lowercase());
        if !seen.insert(key) {
            continue;
        }

        if let Some((owner, existing_forge)) =
            names.insert(repo.repo.clone(), (repo.owner.clone(), forge))
            && (owner != repo.owner || existing_forge != forge)
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
pub(super) fn prepend_factory_definition_clone(setup_commands: &mut Vec<String>) {
    let clone_url = std::env::var(FACTORY_REPO_CLONE_URL_ENV_VAR).unwrap_or_default();
    let clone_dir = std::env::var(FACTORY_REPO_DIR_ENV_VAR).unwrap_or_default();
    prepend_factory_definition_clone_for_values(&clone_url, &clone_dir, setup_commands);
}

fn prepend_factory_definition_clone_for_values(
    clone_url: &str,
    clone_dir: &str,
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
    setup_commands.insert(
        0,
        format!("git clone \"${FACTORY_REPO_CLONE_URL_ENV_VAR}\" \"${FACTORY_REPO_DIR_ENV_VAR}\""),
    );
}

#[allow(clippy::too_many_arguments)]
async fn prepare_environment_impl(
    spawner: &ModelSpawner<TerminalDriver>,
    working_dir: &Path,
    is_sandbox: bool,
    source_repos: &[SourceRepo],
    repository_head_overrides: &[RepositoryHeadOverride],
    remove_repository_origins: bool,
    setup_commands: Vec<String>,
    should_index_codebase: bool,
    repo_channels: Arc<Mutex<HashMap<PathBuf, oneshot::Sender<()>>>>,
    setup_events: SetupClientEventReporter,
    environment_snapshot_reporter: EnvironmentSnapshotReporter,
) -> Result<(), PrepareEnvironmentError> {
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

    let environment_snapshot = if source_repos.is_empty() {
        EnvironmentSnapshot::empty()
    } else {
        setup_events
            .record_result(SetupStep::EnvironmentRepoClone, async {
                clone_checkout_requests(
                    &repository_clone_requests(source_repos, repository_head_overrides)?,
                    working_dir,
                    spawner,
                )
                .await
            })
            .await?
    };
    environment_snapshot_reporter.report(environment_snapshot);

    if !source_repos.is_empty() {
        for repo in source_repos {
            register_cloned_repo(repo, working_dir, is_sandbox, spawner).await?;
            if !is_sandbox && should_index_codebase {
                let receiver = index_repo_codebase(
                    &repo.repo,
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
                cache_setup::setup_caches(cache_root, source_repos, working_dir, spawner),
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

                for command in setup_commands {
                    let command_for_error = command.clone();
                    safe_info!(
                        safe: ("Running setup command"),
                        full: ("Running setup command: {command}")
                    );

                    let exit_code = execute_command(command, spawner).await?;
                    if exit_code != 0.into() {
                        return Err(PrepareEnvironmentError::SetupCommand {
                            command: command_for_error,
                        });
                    }

                    let working_dir_string = working_dir.to_string_lossy().to_string();
                    if let Err(error) = cd_in_terminal(working_dir_string, spawner).await {
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

    let remove_origins_result = if remove_repository_origins {
        remove_repository_origins_from_repos(source_repos, working_dir, spawner).await
    } else {
        Ok(())
    };
    setup_result?;
    remove_origins_result?;

    if should_index_codebase && source_repos.is_empty() {
        log::info!("No repositories to index for codebase context");
    }

    // If there's only one repo in the environment, start the agent in that repo.
    // This way, it doesn't have to locate the correct repo to work on.
    if let Some(repo_name) = single_repo_name(source_repos) {
        safe_info!(
            safe: ("Changing directory into single repository"),
            full: ("Changing directory into single repository: {repo_name}")
        );
        let exit_code = cd_in_terminal(repo_name.clone(), spawner).await?;
        if exit_code != 0.into() {
            return Err(PrepareEnvironmentError::ChangeDirectory { repo_name });
        }
    }
    Ok(())
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
    match repo.code_forge.unwrap_or_default() {
        CodeForge::GitHub => Some(RepositoryForge::GitHub),
        CodeForge::GitLab => Some(RepositoryForge::GitLab),
        CodeForge::None | CodeForge::Unknown => None,
    }
}
fn head_override_matches_repo(head_override: &RepositoryHeadOverride, repo: &SourceRepo) -> bool {
    Some(head_override.code_forge) == repository_forge_for_repo(repo)
        && head_override.repo_owner == repo.owner
        && head_override.repo_name == repo.repo
}

fn head_override_for_repo<'a>(
    overrides: &'a [RepositoryHeadOverride],
    repo: &SourceRepo,
) -> Option<&'a RepositoryHeadOverride> {
    overrides
        .iter()
        .find(|head_override| head_override_matches_repo(head_override, repo))
}

#[derive(Debug, Clone)]
struct RepositoryCloneRequest {
    repo: SourceRepo,
    checkout: Option<RepositoryHeadRef>,
}

fn repository_clone_requests(
    repos: &[SourceRepo],
    overrides: &[RepositoryHeadOverride],
) -> Result<Vec<RepositoryCloneRequest>, PrepareEnvironmentError> {
    repos
        .iter()
        .cloned()
        .map(|repo| {
            // A repository this client can't identify a host for can never
            // clone; fail clearly here rather than attempt one with an empty
            // host, which would otherwise be the only signal something is
            // wrong.
            if repository_forge_for_repo(&repo).is_none() {
                return Err(PrepareEnvironmentError::UnsupportedRepositoryForge {
                    repo_name: format!("{}/{}", repo.owner, repo.repo),
                });
            }
            let checkout = match head_override_for_repo(overrides, &repo) {
                Some(head_override) => Some(head_override.head.clone()),
                None => repo.checkout_ref.clone().map(RepositoryHeadRef::Branch),
            };
            Ok(RepositoryCloneRequest { repo, checkout })
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

fn build_remove_repository_origins_command(
    repos: &[SourceRepo],
    working_dir: &Path,
    shell_type: ShellType,
) -> String {
    let mut script = String::new();
    for repo in repos {
        let repo_path = working_dir.join(&repo.repo);
        let escaped_path =
            shell_escape_single_quotes(&repo_path.to_string_lossy(), ShellType::Bash);
        script.push_str(&format!(
            "if git -C '{escaped_path}' remote get-url origin >/dev/null 2>&1; then\n\
             \tgit -C '{escaped_path}' remote remove origin || exit 1\n\
             fi\n"
        ));
    }
    let escaped_script = shell_escape_single_quotes(&script, shell_type);
    format!("sh -c '{escaped_script}'")
}

async fn remove_repository_origins_from_repos(
    repos: &[SourceRepo],
    working_dir: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(), PrepareEnvironmentError> {
    if repos.is_empty() {
        return Ok(());
    }
    let shell_type = active_shell_type(spawner).await;
    let command = build_remove_repository_origins_command(repos, working_dir, shell_type);
    let output = execute_silent_command(command, spawner).await?;
    if output.success() {
        Ok(())
    } else {
        Err(PrepareEnvironmentError::RemoveRepositoryOrigins)
    }
}

fn build_parallel_clone_command(repos: &[RepositoryCloneRequest], shell_type: ShellType) -> String {
    let mut script = String::from(
        r#"set +e
failed=0
pids=""
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/warp-clone-logs.XXXXXX")"
cleanup_clone_logs() {
  rm -rf "$tmp_dir"
}
trap cleanup_clone_logs EXIT
clone_repo() {
  repo_name="$1"
  repo_url="$2"
  target="$3"
  checkout_ref="$4"
  is_commit_sha="$5"
  if [ "$is_commit_sha" = "1" ]; then
    if [ -e "$target" ]; then
      printf '%s\n' "Checking out $checkout_ref in existing repository $repo_name..."
    else
      printf '%s\n' "Initializing repository $repo_name at $checkout_ref..."
      git init --quiet "$target" || return 1
      git -C "$target" remote add origin "$repo_url" || return 1
    fi
    git -C "$target" fetch --filter=blob:none origin "$checkout_ref" && git -C "$target" checkout --detach FETCH_HEAD
    return
  fi
  if [ -d "$target" ]; then
    printf '%s\n' "Repository directory $target already exists, skipping clone..."
  else
    printf '%s\n' "Cloning repository $repo_name..."
    git clone --filter=blob:none "$repo_url" "$target" || return 1
  fi
  # Pin after clone or reuse: a reused directory may still be on an old ref.
  if [ -n "$checkout_ref" ]; then
    printf '%s\n' "Checking out $checkout_ref in $repo_name..."
    # Fetch leaves the object in FETCH_HEAD; check that out detached so we
    # never prefer a stale local branch with the same name.
    git -C "$target" fetch --filter=blob:none origin "$checkout_ref" && git -C "$target" checkout --detach FETCH_HEAD
  fi
}
"#,
    );

    let mut log_outputs = String::new();
    for (index, request) in repos.iter().enumerate() {
        let repo_name = format!("{}/{}", request.repo.owner, request.repo.repo);
        let repo_url = request.repo.https_clone_url();
        let escaped_repo_name = shell_escape_single_quotes(&repo_name, ShellType::Bash);
        let escaped_repo_url = shell_escape_single_quotes(&repo_url, ShellType::Bash);
        let escaped_target = shell_escape_single_quotes(&request.repo.repo, ShellType::Bash);
        let checkout_ref = request
            .checkout
            .as_ref()
            .map(RepositoryHeadRef::value)
            .unwrap_or_default();
        let escaped_checkout_ref = shell_escape_single_quotes(checkout_ref, ShellType::Bash);
        let is_commit_sha = match request.checkout {
            Some(RepositoryHeadRef::CommitSha(_)) => "1",
            Some(RepositoryHeadRef::Branch(_)) | None => "0",
        };
        let log_var = format!("log_file_{index}");
        script.push_str(&format!(
            "{log_var}=\"$tmp_dir/repo-{index}.log\"\n\
             clone_repo '{escaped_repo_name}' '{escaped_repo_url}' '{escaped_target}' '{escaped_checkout_ref}' '{is_commit_sha}' >\"${log_var}\" 2>&1 &\n"
        ));
        script.push_str("pids=\"$pids $!\"\n");
        log_outputs.push_str(&format!(
            "printf '%s\\n' '===== {escaped_repo_name} ====='\n\
             if [ -s \"${log_var}\" ]; then\n\
             \tcat \"${log_var}\"\n\
             else\n\
             \tprintf '%s\\n' '(no output)'\n\
             fi\n"
        ));
    }

    script.push_str(
        r#"for pid in $pids; do
  if ! wait "$pid"; then
    failed=1
  fi
done
"#,
    );
    script.push_str(&log_outputs);
    script.push_str(
        r#"
exit "$failed"
"#,
    );

    let escaped_script = shell_escape_single_quotes(&script, shell_type);
    format!("sh -c '{escaped_script}'")
}

/// Clone all source repositories to `{working_dir}/{repo.repo}` if they do not already exist.
/// Multiple repositories are cloned in parallel to reduce environment setup time.
pub(super) async fn clone_repos(
    repos: &[SourceRepo],
    working_dir: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(), PrepareEnvironmentError> {
    clone_checkout_requests(
        &repository_clone_requests(repos, &[])?,
        working_dir,
        spawner,
    )
    .await
    .map(|_| ())
}

async fn clone_checkout_requests(
    repos: &[RepositoryCloneRequest],
    working_dir: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<EnvironmentSnapshot, PrepareEnvironmentError> {
    match repos {
        [] => return Ok(EnvironmentSnapshot::empty()),
        [request] => clone_repo(request, working_dir, spawner).await?,
        repos => {
            let shell_type = spawner
                .spawn(|driver, ctx| {
                    driver
                        .active_session_shell_type(ctx)
                        .unwrap_or(ShellType::Bash)
                })
                .await
                .unwrap_or(ShellType::Bash);

            let repo_names = repos
                .iter()
                .map(|request| format!("{}/{}", request.repo.owner, request.repo.repo))
                .collect::<Vec<_>>();
            safe_info!(
                safe: ("Cloning repositories via terminal"),
                full: ("Cloning repositories via terminal: {}", repo_names.join(", "))
            );

            let command = build_parallel_clone_command(repos, shell_type);
            let exit_code = execute_command(command, spawner).await?;
            if exit_code != 0.into() {
                return Err(PrepareEnvironmentError::CloneRepo {
                    repo_name: repo_names.join(", "),
                });
            }

            safe_info!(
                safe: ("Successfully cloned repositories"),
                full: ("Successfully cloned repositories: {}", repo_names.join(", "))
            );
        }
    }
    Ok(capture_environment_snapshot(repos, working_dir, spawner).await)
}

/// Clone a source repository to `{working_dir}/{repo.repo}` if it does not already exist.
/// This only performs the clone -- it does NOT register the repo with `DetectedRepositories`.
#[tracing::instrument(skip_all, err, fields(tags.cloud_agent = true, repo = %request.repo))]
async fn clone_repo(
    request: &RepositoryCloneRequest,
    working_dir: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(), PrepareEnvironmentError> {
    let repo = &request.repo;
    let repo_name = format!("{}/{}", repo.owner, repo.repo);
    let repo_url = repo.https_clone_url();
    // Get the session's shell type for proper escaping, falling back to Bash
    // when the session is not yet bootstrapped or the spawn fails.
    let shell_type = spawner
        .spawn(|driver, ctx| {
            driver
                .active_session_shell_type(ctx)
                .unwrap_or(ShellType::Bash)
        })
        .await
        .unwrap_or(ShellType::Bash);
    let escaped_url = shell_escape_single_quotes(&repo_url, shell_type);
    let repo_dir = working_dir.join(&repo.repo);
    let commit_sha = match &request.checkout {
        Some(RepositoryHeadRef::CommitSha(commit_sha)) => Some(commit_sha.as_str()),
        Some(RepositoryHeadRef::Branch(_)) | None => None,
    };
    // Always ask the session whether the repo dir already exists, rather
    // than stat'ing from the host. The session knows about sandbox-only
    // paths, and this goes through the silent executor so `test -d` is
    // not added to the user-visible blocklist. Pass the absolute path
    // explicitly so the probe doesn't rely on the session's CWD.
    let dir_exists = terminal_directory_exists(&repo_dir.to_string_lossy(), spawner).await?;

    if let Some(commit_sha) = commit_sha {
        if !dir_exists {
            safe_info!(
                safe: ("Initializing repository at commit via terminal"),
                full: ("Initializing repository via terminal: {repo_name} at {commit_sha}")
            );
            let escaped_dir = shell_escape_single_quotes(&repo_dir.to_string_lossy(), shell_type);
            let init_command = format!(
                "git init --quiet '{escaped_dir}' && git -C '{escaped_dir}' remote add origin '{escaped_url}'"
            );
            let exit_code = execute_command(init_command, spawner).await?;
            if exit_code != 0.into() {
                return Err(PrepareEnvironmentError::CloneRepo {
                    repo_name: repo_name.clone(),
                });
            }
        }
    } else if dir_exists {
        safe_warn!(
            safe: ("We already have a directory with the same repository name in the terminal working directory, skipping clone..."),
            full: (
            "We already have a directory with the name {} in the terminal working directory, skipping clone...",
            repo.repo)
        );
    } else {
        safe_info!(
            safe: ("Cloning repository via terminal"),
            full: ("Cloning repository via terminal: {repo_name}")
        );

        // We do a blobless partial clone here to speed up environment setup
        // time while still keeping trees local, so path-limited history and
        // blame stay fully local instead of lazily refetching from the
        // promisor remote.
        let command = format!("git clone --filter=blob:none '{escaped_url}'");
        let exit_code = execute_command(command, spawner).await?;
        if exit_code != 0.into() {
            return Err(PrepareEnvironmentError::CloneRepo {
                repo_name: repo_name.clone(),
            });
        }

        safe_info!(
            safe: ("Successfully cloned repository"),
            full: ("Successfully cloned: {repo_name}")
        );
    }

    // Pin after clone or reuse when a ref was requested. A reused directory may
    // still be on an old default-branch tip, and a checkout_ref (SHA, branch,
    // or tag) may not have existed yet, or may have moved, by the time the
    // clone ran — fetch the ref, then detach to FETCH_HEAD.
    // When checkout_ref is unset, leave an existing directory untouched.
    if let Some(command) = checkout_command_for(request, working_dir, shell_type) {
        let checkout_ref = request
            .checkout
            .as_ref()
            .map(RepositoryHeadRef::value)
            .unwrap_or_default();
        safe_info!(
            safe: ("Checking out pinned ref for repository"),
            full: ("Checking out {checkout_ref} for {repo_name}")
        );
        let exit_code = execute_command(command, spawner).await?;
        checkout_result(&repo_name, checkout_ref, exit_code)?;

        safe_info!(
            safe: ("Successfully checked out pinned ref"),
            full: ("Successfully checked out {checkout_ref} for {repo_name}")
        );
    }

    Ok(())
}

async fn capture_environment_snapshot(
    repos: &[RepositoryCloneRequest],
    working_dir: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> EnvironmentSnapshot {
    if repos.is_empty() {
        return EnvironmentSnapshot::empty();
    }
    let command = build_resolved_head_command(repos, working_dir);
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

/// Build the `git fetch` + `git checkout` command that pins `request`'s clone at
/// its checkout, or `None` when the repo has no ref to pin.
///
/// The requested ref (commit SHA, branch, or tag) may not have existed yet,
/// or may have moved, by the time the clone ran: fetch it first, then check
/// out the resulting `FETCH_HEAD` detached. Checking out the original ref
/// name can prefer a stale local branch or fail when the object only landed
/// in `FETCH_HEAD`. Detached HEAD is expected and fine — trials never merge.
fn checkout_command_for(
    request: &RepositoryCloneRequest,
    working_dir: &Path,
    shell_type: ShellType,
) -> Option<String> {
    let checkout_ref = request.checkout.as_ref()?.value();
    let repo_dir = working_dir.join(&request.repo.repo);
    let escaped_dir = shell_escape_single_quotes(&repo_dir.to_string_lossy(), shell_type);
    let escaped_ref = shell_escape_single_quotes(checkout_ref, shell_type);
    Some(format!(
        "git -C '{escaped_dir}' fetch --filter=blob:none origin '{escaped_ref}' && \
         git -C '{escaped_dir}' checkout --detach FETCH_HEAD"
    ))
}

/// Map a checkout command's exit code onto the environment-prep result,
/// surfacing a non-zero exit (fetch or checkout failing) as `CheckoutFailed`
/// rather than silently leaving the clone on the default branch.
fn checkout_result(
    repo_name: &str,
    checkout_ref: &str,
    exit_code: ExitCode,
) -> Result<(), PrepareEnvironmentError> {
    if exit_code == 0.into() {
        Ok(())
    } else {
        Err(PrepareEnvironmentError::CheckoutFailed {
            repo_name: repo_name.to_string(),
            checkout_ref: checkout_ref.to_string(),
        })
    }
}

/// Register a cloned source repository with `DetectedRepositories` so that the
/// skill watcher and other repo-aware subsystems can discover it.
#[tracing::instrument(skip_all, err, fields(tags.cloud_agent = true, repo = %repo, is_sandbox = is_sandbox))]
pub(super) async fn register_cloned_repo(
    repo: &SourceRepo,
    working_dir: &Path,
    is_sandbox: bool,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(), PrepareEnvironmentError> {
    let repo_dir = working_dir.join(&repo.repo);

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

/// Execute a command in the context of a terminal session.
async fn execute_command(
    command: String,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<ExitCode, PrepareEnvironmentError> {
    spawner
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
        })?
        .await
        .map_err(|error| match error {
            AgentDriverError::InvalidRuntimeState => PrepareEnvironmentError::InvalidRuntimeState,
            source => PrepareEnvironmentError::TerminalDriver { source },
        })
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

async fn terminal_directory_exists(
    path: &str,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<bool, PrepareEnvironmentError> {
    let path = path.to_owned();
    let output = spawner
        .spawn(move |driver, ctx| {
            // Fall back to Bash if the session's shell type isn't known yet
            // (e.g. pre-bootstrap). Bash-style escaping is a safe default for
            // every POSIX shell we currently support.
            let shell_type = driver
                .active_session_shell_type(ctx)
                .unwrap_or(ShellType::Bash);
            let escaped = shell_escape_single_quotes(&path, shell_type);
            let command = format!("test -d '{escaped}'");
            driver.execute_silent_command(command, ctx)
        })
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
