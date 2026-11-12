use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, TimeDelta, Utc};
use command::blocking::Command;
use instant::Instant;
use warp_graphql::ai::PlatformErrorCode;
use warp_graphql::client::GraphQLError;
use warp_graphql::queries::task_restricted_git_credentials::{
    GitHubCredentialMode, RestrictedTaskGitCredentialsOutput,
};

use super::{
    GIT_CREDENTIALS_BOOTSTRAP_BACKOFF, GIT_CREDENTIALS_REFRESH_INTERVAL, GITHUB_HOST,
    GitCredential, READ_ONLY_CREDENTIAL_HELPER, TaskGitCredentialsError,
    ensure_workload_token_available, gh_config_dir, home_dir, record_host_identities,
    unique_credentials_by_host, write_git_credentials,
};
use crate::server::retry_strategies::with_retry;
use crate::server::server_api::ai::AIClient;

const AUTH_ENV: &[&str] = &[
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GITHUB_ACCESS_TOKEN",
    "GITHUB_PERSONAL_ACCESS_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
];
const FRESH_SANDBOX_REQUIRED: &str =
    "Restricted GitHub credentials require a fresh isolated managed HOME; start a new sandbox";
const REFRESH_FALLBACK_INTERVAL: Duration = Duration::from_secs(60);
const REFRESH_EXPIRY_BUFFER: TimeDelta = TimeDelta::minutes(10);

const GIT_OVERRIDE_ENV: &[&str] = &[
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_SYSTEM",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
];

struct ManagedStore {
    task_id: String,
    directory: PathBuf,
    refresh_at: Option<Instant>,
    expires_at: Option<Instant>,
}

fn is_temporary_failure(error: &TaskGitCredentialsError) -> bool {
    match error {
        TaskGitCredentialsError::Platform { info, .. } => {
            info.retryable
                && !matches!(info.http_status, Some(401 | 403))
                && matches!(
                    info.code,
                    PlatformErrorCode::InternalError
                        | PlatformErrorCode::ResourceUnavailable
                        | PlatformErrorCode::AgentStreamNetworkError
                )
        }
        TaskGitCredentialsError::Request(error) => {
            for cause in error.chain() {
                if let Some(error) = cause.downcast_ref::<GraphQLError>() {
                    return match error {
                        GraphQLError::HttpError { status, .. } => {
                            matches!(status.as_u16(), 408 | 429 | 500..=599)
                        }
                        GraphQLError::RequestError(error) | GraphQLError::ResponseError(error) => {
                            error.is_timeout() || error.is_connect() || error.is_body()
                        }
                        GraphQLError::StagingAccessBlocked | GraphQLError::IapChallengeBlocked => {
                            false
                        }
                    };
                }
                if let Some(error) = cause.downcast_ref::<reqwest::Error>() {
                    return match error.status() {
                        Some(status) => matches!(status.as_u16(), 408 | 429 | 500..=599),
                        None => error.is_timeout() || error.is_connect() || error.is_body(),
                    };
                }
            }
            false
        }
        TaskGitCredentialsError::Unstructured { .. } => false,
    }
}

struct ValidatedCredentials {
    credentials: Vec<GitCredential>,
    refresh_at: Instant,
    expires_at: Option<Instant>,
    failed_hosts: Vec<String>,
}

static MANAGED_STORE: Mutex<Option<ManagedStore>> = Mutex::new(None);

pub(crate) fn restricted_github_mode() -> Result<bool> {
    match std::env::var("WARP_GITHUB_CREDENTIAL_MODE") {
        Ok(value) => Ok(value == "restricted_installation"),
        Err(std::env::VarError::NotPresent | std::env::VarError::NotUnicode(_)) => Ok(false),
    }
}

fn task_dir(task_id: &str) -> Result<PathBuf> {
    ensure!(
        !task_id.is_empty()
            && task_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "Invalid task ID for restricted GitHub store",
    );
    Ok(home_dir()?.join(".warp").join("github-git").join(task_id))
}

fn private_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{FRESH_SANDBOX_REQUIRED}"
    );
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_store(directory: &Path, content: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{FRESH_SANDBOX_REQUIRED}"
    );
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(content.as_bytes())?;
    temporary
        .persist(directory.join("credentials"))
        .map_err(|error| error.error)?;
    Ok(())
}

fn git_config(args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(["config", "--global"])
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "Could not configure restricted GitHub credentials"
    );
    Ok(())
}

fn prepare(task_id: &str) -> Result<PathBuf> {
    let mut managed = MANAGED_STORE
        .lock()
        .map_err(|_| anyhow::anyhow!("GitHub store lock failed"))?;
    ensure!(managed.is_none(), "{FRESH_SANDBOX_REQUIRED}");
    let home = home_dir()?;
    ensure!(
        !GIT_OVERRIDE_ENV
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty())),
        "{FRESH_SANDBOX_REQUIRED}",
    );
    ensure!(
        std::env::var_os("GIT_CONFIG_GLOBAL")
            .is_none_or(|path| path == home.join(".gitconfig").as_os_str()),
        "{FRESH_SANDBOX_REQUIRED}",
    );
    ensure!(
        !AUTH_ENV
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty())),
        "{FRESH_SANDBOX_REQUIRED}",
    );
    for path in [
        home.join(".git-credentials"),
        home.join(".netrc"),
        home.join(".config/git/credentials"),
        gh_config_dir(&home).join("hosts.yml"),
        home.join(".config/gh/hosts.yml"),
    ] {
        ensure!(
            fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound),
            "{FRESH_SANDBOX_REQUIRED}"
        );
    }
    for path in [home.join(".gitconfig"), home.join(".config/git/config")] {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "{FRESH_SANDBOX_REQUIRED}"
            );
        }
    }
    let helpers = Command::new("git")
        .args([
            "config",
            "--get-regexp",
            r"^(credential\..*helper|http\..*extraheader|http\..*cookiefile)$",
        ])
        .output()?;
    ensure!(helpers.status.code() == Some(1), "{FRESH_SANDBOX_REQUIRED}");
    let directory = task_dir(task_id)?;
    ensure!(!directory.try_exists()?, "{FRESH_SANDBOX_REQUIRED}");
    private_directory(&home.join(".warp"))?;
    private_directory(&home.join(".warp/github-git"))?;
    private_directory(&directory)?;
    private_directory(&directory.join("gh"))?;
    write_store(&directory, "")?;
    *managed = Some(ManagedStore {
        task_id: task_id.into(),
        directory: directory.clone(),
        refresh_at: None,
        expires_at: None,
    });
    Ok(directory)
}

fn install_helper(directory: &Path, credentials: &[GitCredential]) -> Result<()> {
    let store = directory.join("credentials");
    let store = store
        .to_str()
        .context("GitHub credential path is not UTF-8")?;
    let quoted = format!("'{}'", store.replace('\'', "'\\''"));
    let helper = format!(
        "!f() {{ test \"$1\" = get && exec git credential-store --file={quoted} get; }}; f"
    );
    git_config(&["credential.helper", READ_ONLY_CREDENTIAL_HELPER])?;
    git_config(&["--replace-all", "credential.https://github.com.helper", ""])?;
    git_config(&["--add", "credential.https://github.com.helper", &helper])?;
    git_config(&[
        "--add",
        "url.https://github.com/.insteadOf",
        "git@github.com:",
    ])?;
    git_config(&[
        "--add",
        "url.https://github.com/.insteadOf",
        "ssh://git@github.com/",
    ])?;
    for credential in credentials
        .iter()
        .filter(|credential| credential.host != GITHUB_HOST)
    {
        let host = &credential.host;
        git_config(&[
            "--add",
            &format!("url.https://{host}/.insteadOf"),
            &format!("git@{host}:"),
        ])?;
        git_config(&[
            "--add",
            &format!("url.https://{host}/.insteadOf"),
            &format!("ssh://git@{host}/"),
        ])?;
    }
    Ok(())
}

fn validate_response(
    response: RestrictedTaskGitCredentialsOutput,
    now: DateTime<Utc>,
    received_at: Instant,
) -> Result<ValidatedCredentials> {
    ensure!(
        response.github_credential_mode == GitHubCredentialMode::RestrictedInstallation,
        "Restricted GitHub credential mode was not returned",
    );
    ensure!(
        response
            .failed_hosts
            .iter()
            .all(|host| matches!(host.as_str(), GITHUB_HOST | "gitlab.com" | "dev.azure.com")),
        "Unexpected failed forge in restricted credential response"
    );
    let mut credentials = Vec::new();
    let mut refresh_interval = GIT_CREDENTIALS_REFRESH_INTERVAL;
    let mut github_expires_at = None;
    for credential in response.credentials {
        ensure!(
            !response.failed_hosts.contains(&credential.host),
            "Restricted credential response both succeeded and failed for a forge"
        );
        ensure!(
            matches!(
                credential.host.as_str(),
                GITHUB_HOST | "gitlab.com" | "dev.azure.com"
            ),
            "Unexpected forge in restricted credential response",
        );
        if credential.host == GITHUB_HOST {
            ensure!(
                credential
                    .expires_at
                    .is_some_and(|expiry| expiry.utc() > now + TimeDelta::minutes(5))
                    && credential
                        .repository_scope
                        .as_ref()
                        .is_some_and(|scope| !scope.is_empty())
                    && credential
                        .username
                        .as_ref()
                        .is_some_and(|name| !name.is_empty())
                    && credential
                        .email
                        .as_ref()
                        .is_some_and(|email| !email.is_empty())
                    && !credential.token.is_empty()
                    && !credential.token.contains(['\n', '\r', '@', '/']),
                "Restricted GitHub credential is missing valid expiry, repository scope, or bot identity",
            );
            let expires_at = credential
                .expires_at
                .context("Missing GitHub credential expiry")?;
            github_expires_at = Some(received_at + (expires_at.utc() - now).to_std()?);
            let interval = (expires_at.utc() - now - REFRESH_EXPIRY_BUFFER)
                .to_std()
                .unwrap_or_default()
                .max(REFRESH_FALLBACK_INTERVAL);
            refresh_interval = refresh_interval.min(interval);
        } else {
            // Other forges do not yet supply expiry, so keep their existing refresh cadence.
            refresh_interval = REFRESH_FALLBACK_INTERVAL;
        }
        credentials.push(GitCredential {
            token: credential.token,
            username: credential.username,
            email: credential.email,
            host: credential.host,
        });
    }
    let credentials = unique_credentials_by_host(&credentials)?;
    if credentials.is_empty() || !response.failed_hosts.is_empty() {
        refresh_interval = REFRESH_FALLBACK_INTERVAL;
    }
    Ok(ValidatedCredentials {
        credentials,
        refresh_at: received_at + refresh_interval,
        expires_at: github_expires_at,
        failed_hosts: response.failed_hosts,
    })
}

async fn fetch(
    task_id: &str,
    ai_client: &Arc<dyn AIClient>,
    use_factory_repositories: bool,
    accepts_partial_refresh: bool,
) -> Result<ValidatedCredentials, TaskGitCredentialsError> {
    ensure_workload_token_available()?;
    let token = warp_isolation_platform::issue_workload_token(Some(Duration::from_secs(300)))
        .await
        .map_err(|error| TaskGitCredentialsError::Request(error.into()))?
        .token;
    let response = ai_client
        .get_restricted_task_git_credentials(
            task_id.to_owned(),
            token,
            accepts_partial_refresh,
            use_factory_repositories,
        )
        .await?;
    if !accepts_partial_refresh && !response.failed_hosts.is_empty() {
        return Err(TaskGitCredentialsError::Unstructured {
            message: "Restricted credential bootstrap was incomplete".into(),
        });
    }
    validate_response(response, Utc::now(), Instant::now()).map_err(|error| {
        TaskGitCredentialsError::Unstructured {
            message: error.to_string(),
        }
    })
}

fn apply(store: &mut ManagedStore, response: &ValidatedCredentials) -> Result<()> {
    ensure!(
        response
            .expires_at
            .is_none_or(|expiry| expiry > Instant::now()),
        "Restricted GitHub credential expired before installation"
    );
    let content = response
        .credentials
        .iter()
        .find(|credential| credential.host == GITHUB_HOST)
        .map(|credential| format!("https://x-access-token:{}@github.com\n", credential.token))
        .unwrap_or_default();
    write_store(&store.directory, &content)?;
    let other = response
        .credentials
        .iter()
        .filter(|credential| credential.host != GITHUB_HOST)
        .cloned()
        .collect::<Vec<_>>();
    write_git_credentials(&other)?;
    store.refresh_at = Some(response.refresh_at);
    store.expires_at = response.expires_at;
    Ok(())
}

pub(crate) async fn bootstrap_restricted_credentials(
    task_id: &str,
    ai_client: &Arc<dyn AIClient>,
    use_factory_repositories: bool,
) -> Result<(), TaskGitCredentialsError> {
    let directory = prepare(task_id).map_err(TaskGitCredentialsError::Request)?;
    let result = async {
        let response = with_retry(
            "Restricted Git credentials bootstrap",
            || fetch(task_id, ai_client, use_factory_repositories, false),
            super::is_retryable,
            |delay| async move {
                warpui::r#async::Timer::after(delay).await;
            },
            |attempts_made| {
                GIT_CREDENTIALS_BOOTSTRAP_BACKOFF
                    .get(attempts_made)
                    .copied()
            },
        )
        .await?;
        install_helper(&directory, &response.credentials)
            .map_err(TaskGitCredentialsError::Request)?;
        if let Some(github) = response
            .credentials
            .iter()
            .find(|credential| credential.host == GITHUB_HOST)
        {
            git_config(&["user.name", github.username.as_deref().unwrap_or_default()])
                .map_err(TaskGitCredentialsError::Request)?;
            git_config(&["user.email", github.email.as_deref().unwrap_or_default()])
                .map_err(TaskGitCredentialsError::Request)?;
        }
        record_host_identities(&response.credentials);
        with_store(task_id, |store| apply(store, &response))
            .map_err(TaskGitCredentialsError::Request)
    }
    .await;
    if result.is_err() {
        clear_restricted_credentials(task_id).map_err(TaskGitCredentialsError::Request)?;
    }
    result
}

pub(crate) async fn refresh_restricted_credentials(
    task_id: &str,
    ai_client: &Arc<dyn AIClient>,
    use_factory_repositories: bool,
) -> Result<(), TaskGitCredentialsError> {
    apply_refresh(
        task_id,
        fetch(task_id, ai_client, use_factory_repositories, true).await,
    )
}

fn apply_refresh(
    task_id: &str,
    fetched: Result<ValidatedCredentials, TaskGitCredentialsError>,
) -> Result<(), TaskGitCredentialsError> {
    match fetched {
        Ok(response) => {
            if let Err(error) = with_store(task_id, |store| apply(store, &response)) {
                clear_restricted_credentials(task_id).map_err(TaskGitCredentialsError::Request)?;
                return Err(TaskGitCredentialsError::Request(error));
            }
            if !response.failed_hosts.is_empty() {
                return Err(TaskGitCredentialsError::Request(anyhow::anyhow!(
                    "Restricted credential refresh was incomplete"
                )));
            }
            Ok(())
        }
        Err(error) => {
            if is_temporary_failure(&error) {
                with_store(task_id, |store| {
                    store.refresh_at = None;
                    if store
                        .expires_at
                        .is_none_or(|expiry| expiry <= Instant::now())
                    {
                        write_store(&store.directory, "")?;
                        store.expires_at = None;
                    }
                    Ok(())
                })
                .map_err(TaskGitCredentialsError::Request)?;
            } else {
                clear_restricted_credentials(task_id).map_err(TaskGitCredentialsError::Request)?;
            }
            Err(error)
        }
    }
}

pub(crate) fn clear_restricted_credentials(task_id: &str) -> Result<()> {
    with_store(task_id, |store| {
        store.refresh_at = None;
        write_store(&store.directory, "")?;
        store.expires_at = None;
        Ok(())
    })
}

fn expire_credentials(task_id: &str, now: Instant) -> Result<Duration> {
    with_store(task_id, |store| {
        if let Some(expiry) = store.expires_at {
            if expiry > now {
                return Ok(expiry - now);
            }
            write_store(&store.directory, "")?;
            store.expires_at = None;
            store.refresh_at = None;
        }
        Ok(REFRESH_FALLBACK_INTERVAL)
    })
}

pub(super) async fn expiry_loop(task_id: &str) {
    loop {
        let delay = expire_credentials(task_id, Instant::now()).unwrap_or_else(|error| {
            log::warn!("Could not expire restricted GitHub credentials: {error}");
            REFRESH_FALLBACK_INTERVAL
        });
        warpui::r#async::Timer::after(delay.min(REFRESH_FALLBACK_INTERVAL)).await;
    }
}

pub(super) fn refresh_delay(task_id: &str, now: Instant) -> Duration {
    with_store(task_id, |store| {
        Ok(store
            .refresh_at
            .map(|deadline| deadline.saturating_duration_since(now)))
    })
    .ok()
    .flatten()
    .unwrap_or(REFRESH_FALLBACK_INTERVAL)
}

fn with_store<T>(task_id: &str, action: impl FnOnce(&mut ManagedStore) -> Result<T>) -> Result<T> {
    let mut managed = MANAGED_STORE
        .lock()
        .map_err(|_| anyhow::anyhow!("GitHub store lock failed"))?;
    let managed = managed
        .as_mut()
        .filter(|managed| managed.task_id == task_id)
        .context("No active restricted GitHub store for this task")?;
    action(managed)
}

pub(crate) fn close_restricted_credentials(task_id: &str) -> Result<()> {
    let mut managed = MANAGED_STORE
        .lock()
        .map_err(|_| anyhow::anyhow!("GitHub store lock failed"))?;
    if let Some(store) = managed
        .as_mut()
        .filter(|managed| managed.task_id == task_id)
    {
        store.refresh_at = None;
        write_store(&store.directory, "")?;
        *managed = None;
    }
    Ok(())
}

pub(crate) fn restricted_child_environment(task_id: &str) -> Result<HashMap<OsString, OsString>> {
    let mut env = AUTH_ENV
        .iter()
        .map(|name| (OsString::from(*name), OsString::new()))
        .collect::<HashMap<_, _>>();
    env.insert(
        "GH_CONFIG_DIR".into(),
        task_dir(task_id)?.join("gh").into_os_string(),
    );
    env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    env.insert("HOME".into(), home_dir()?.into_os_string());
    env.insert(
        "GIT_CONFIG_GLOBAL".into(),
        home_dir()?.join(".gitconfig").into_os_string(),
    );
    env.insert("GIT_CONFIG_COUNT".into(), "0".into());
    env.insert(
        "WARP_GITHUB_CREDENTIAL_MODE".into(),
        "restricted_installation".into(),
    );
    Ok(env)
}

pub(crate) fn validate_restricted_child_environment(
    env: &HashMap<OsString, OsString>,
) -> Result<()> {
    ensure!(
        !GIT_OVERRIDE_ENV.iter().any(|name| env
            .get(&OsString::from(*name))
            .is_some_and(|value| !value.is_empty())),
        "Restricted GitHub mode does not allow task-provided Git configuration overrides",
    );
    Ok(())
}

#[cfg(test)]
#[path = "restricted_tests.rs"]
mod tests;
