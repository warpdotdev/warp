use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::future::Future;
use std::io::{ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, TimeDelta, Utc};
use command::blocking::Command;
use futures::FutureExt as _;
use instant::Instant;
use warp_graphql::ai::PlatformErrorCode;
use warp_graphql::client::GraphQLError;
use warp_graphql::queries::task_restricted_git_credentials::{
    GitHubCredentialMode, RestrictedTaskGitCredentialsOutput,
};
use warpui::r#async::Timer;

use super::{
    GIT_CREDENTIALS_BOOTSTRAP_BACKOFF, GIT_CREDENTIALS_REFRESH_INTERVAL, GITHUB_HOST,
    GitCredential, READ_ONLY_CREDENTIAL_HELPER, TaskGitCredentialsError,
    ensure_workload_token_available, home_dir, is_retryable, record_host_identities,
    unique_credentials_by_host, write_git_credentials,
};
use crate::server::retry_strategies::with_retry;
use crate::server::server_api::ai::AIClient;
use crate::server::server_api::presigned_upload::HttpStatusError;

const AUTH_ENV: &[&str] = &[
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GITHUB_ACCESS_TOKEN",
    "GITHUB_PERSONAL_ACCESS_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
];
pub(super) const REFRESH_FALLBACK_INTERVAL: Duration = Duration::from_secs(60);
const REFRESH_EXPIRY_BUFFER: TimeDelta = TimeDelta::minutes(10);

const GIT_OVERRIDE_ENV: &[&str] = &[
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_SYSTEM",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
];

pub(crate) struct CredentialStoreGuard {
    task_id: String,
    armed: bool,
}

impl CredentialStoreGuard {
    pub(crate) fn new(task_id: String) -> Self {
        Self {
            task_id,
            armed: true,
        }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Could not remove managed GitHub credentials")]
struct CredentialStoreCleanupError(#[source] anyhow::Error);

pub(crate) fn is_fatal_cleanup_error(error: &TaskGitCredentialsError) -> bool {
    match error {
        TaskGitCredentialsError::Request(error) => error
            .chain()
            .any(|cause| cause.is::<CredentialStoreCleanupError>()),
        TaskGitCredentialsError::Platform { .. } | TaskGitCredentialsError::Unstructured { .. } => {
            false
        }
    }
}

impl Drop for CredentialStoreGuard {
    fn drop(&mut self) {
        if self.armed
            && let Err(error) = close_restricted_credentials(&self.task_id)
        {
            log::error!("Failed to close managed GitHub credentials: {error:#}");
        }
    }
}

struct ManagedStore {
    task_id: String,
    directory: PathBuf,
    refresh_at: Option<Instant>,
    expires_at: Option<Instant>,
}

struct ValidatedCredentials {
    credentials: Vec<GitCredential>,
    refresh_at: Instant,
    expires_at: Option<Instant>,
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
        "Managed GitHub path is not a real directory: {}",
        path.display()
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
        "Unsafe managed GitHub credential path"
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
        .args(["config", "--file"])
        .arg(home_dir()?.join(".gitconfig"))
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
    ensure!(
        managed.is_none(),
        "A managed GitHub credential store is already active"
    );
    let home = home_dir()?;
    ensure!(
        !GIT_OVERRIDE_ENV
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty())),
        "Restricted GitHub mode does not allow inherited Git configuration overrides",
    );
    ensure!(
        std::env::var_os("GIT_CONFIG_GLOBAL")
            .is_none_or(|path| path == home.join(".gitconfig").as_os_str()),
        "Unsafe managed GitHub credential path",
    );
    for path in [home.join(".gitconfig"), home.join(".config/git/config")] {
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Managed Git configuration is not a regular file: {}",
                path.display()
            );
        }
    }
    ensure!(
        valid_config_count(std::env::var_os("GIT_CONFIG_COUNT").as_deref()),
        "Restricted GitHub mode does not allow nonzero GIT_CONFIG_COUNT"
    );
    for name in [".netrc", "_netrc"] {
        ensure!(
            fs::symlink_metadata(home.join(name))
                .is_err_and(|error| error.kind() == ErrorKind::NotFound),
            "Restricted GitHub mode does not allow inherited netrc credentials"
        );
    }
    let http_auth = Command::new("git")
        .args([
            "config",
            "--get-regexp",
            r"^http\..*(extraheader|cookiefile)$",
        ])
        .output()?;
    ensure!(
        http_auth.status.code() == Some(1),
        "Restricted GitHub mode does not allow inherited HTTP authentication"
    );
    let directory = task_dir(task_id)?;
    ensure!(
        fs::symlink_metadata(&directory).is_err_and(|error| error.kind() == ErrorKind::NotFound),
        "Managed GitHub task directory already exists: {}",
        directory.display()
    );
    private_directory(&home.join(".warp"))?;
    private_directory(&home.join(".warp/github-git"))?;
    private_directory(&directory)?;
    private_directory(&directory.join("gh"))?;
    write_store(&directory, "")?;
    *managed = Some(ManagedStore {
        task_id: task_id.to_owned(),
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
    git_config(&[
        "--replace-all",
        "credential.helper",
        READ_ONLY_CREDENTIAL_HELPER,
    ])?;
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
        response.failed_hosts.is_empty(),
        "Restricted credential refresh was incomplete"
    );
    let mut credentials = Vec::new();
    let mut refresh_interval = GIT_CREDENTIALS_REFRESH_INTERVAL;
    let mut expires_at = None;
    for credential in response.credentials {
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
            let credential_expiry = credential
                .expires_at
                .context("Missing GitHub credential expiry")?;
            let remaining = (credential_expiry.utc() - now).to_std()?;
            expires_at = Some(expires_at.map_or(received_at + remaining, |old: Instant| {
                old.min(received_at + remaining)
            }));
            let interval = (credential_expiry.utc() - now - REFRESH_EXPIRY_BUFFER)
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
    if credentials.is_empty() {
        refresh_interval = REFRESH_FALLBACK_INTERVAL;
    }
    Ok(ValidatedCredentials {
        credentials,
        refresh_at: received_at + refresh_interval,
        expires_at,
    })
}

async fn fetch(
    task_id: &str,
    ai_client: &Arc<dyn AIClient>,
    accepts_partial_refresh: bool,
    use_factory_repositories: bool,
) -> Result<RestrictedTaskGitCredentialsOutput, TaskGitCredentialsError> {
    ensure_workload_token_available()?;
    let token = warp_isolation_platform::issue_workload_token(Some(Duration::from_secs(300)))
        .await
        .map_err(|error| TaskGitCredentialsError::Request(error.into()))?
        .token;
    ai_client
        .get_restricted_task_git_credentials(
            task_id.to_owned(),
            token,
            accepts_partial_refresh,
            use_factory_repositories,
        )
        .await
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
    if let Err(error) = write_store(&store.directory, &content) {
        store.refresh_at = None;
        if let Err(cleanup) = clear_store(store) {
            log::error!(
                "Failed to clear GitHub credentials after a store write failure: {cleanup:#}"
            );
            return Err(cleanup.context(error));
        }
        return Err(error);
    }
    store.refresh_at = Some(response.refresh_at);
    store.expires_at = response.expires_at;
    let other = response
        .credentials
        .iter()
        .filter(|credential| credential.host != GITHUB_HOST)
        .cloned()
        .collect::<Vec<_>>();
    write_git_credentials(&other)
}

fn install(task_id: &str, response: &ValidatedCredentials) -> Result<()> {
    with_store(task_id, |store| {
        install_helper(&store.directory, &response.credentials)?;
        if let Some(github) = response
            .credentials
            .iter()
            .find(|credential| credential.host == GITHUB_HOST)
        {
            git_config(&["user.name", github.username.as_deref().unwrap_or_default()])?;
            git_config(&["user.email", github.email.as_deref().unwrap_or_default()])?;
        }
        record_host_identities(&response.credentials);
        apply(store, response)
    })
}

pub(crate) async fn bootstrap_restricted_credentials(
    task_id: &str,
    ai_client: &Arc<dyn AIClient>,
    use_factory_repositories: bool,
) -> Result<(), TaskGitCredentialsError> {
    prepare(task_id).map_err(TaskGitCredentialsError::Request)?;
    let mut guard = CredentialStoreGuard::new(task_id.to_owned());
    let result = async {
        // Preparation is single-use. Retry only the raw fetch, never validation or installation.
        let response = with_retry(
            "Restricted Git credentials bootstrap",
            || fetch(task_id, ai_client, false, use_factory_repositories),
            is_retryable,
            |delay| async move {
                Timer::after(delay).await;
            },
            |attempt| GIT_CREDENTIALS_BOOTSTRAP_BACKOFF.get(attempt).copied(),
        )
        .await?;
        let response = validate_response(response, Utc::now(), Instant::now())
            .map_err(TaskGitCredentialsError::Request)?;
        install(task_id, &response).map_err(TaskGitCredentialsError::Request)?;
        Ok(())
    }
    .await;
    if result.is_ok() {
        guard.disarm();
    }
    result
}

pub(crate) fn is_transient_credential_error(error: &TaskGitCredentialsError) -> bool {
    match error {
        TaskGitCredentialsError::Platform { info, .. } => {
            info.retryable
                && matches!(
                    info.code,
                    PlatformErrorCode::InternalError
                        | PlatformErrorCode::ResourceUnavailable
                        | PlatformErrorCode::AgentStreamNetworkError
                )
                && info
                    .http_status
                    .is_none_or(|status| matches!(status, 408 | 429 | 500..=599))
        }
        TaskGitCredentialsError::Request(error) => is_transient_request_error(error),
        TaskGitCredentialsError::Unstructured { .. } => false,
    }
}

pub(crate) fn is_transient_request_error(error: &anyhow::Error) -> bool {
    let mut transient = false;
    for cause in error.chain() {
        if let Some(error) = cause.downcast_ref::<std::io::Error>() {
            transient |= matches!(
                error.kind(),
                ErrorKind::TimedOut
                    | ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::NotConnected
            );
        }
        if let Some(error) = cause.downcast_ref::<TaskGitCredentialsError>() {
            if !is_transient_credential_error(error) {
                return false;
            }
            transient = true;
        }
        if let Some(error) = cause.downcast_ref::<GraphQLError>() {
            match error {
                GraphQLError::HttpError { status, .. } => {
                    if !matches!(status.as_u16(), 408 | 429 | 500..=599) {
                        return false;
                    }
                    transient = true;
                }
                GraphQLError::RequestError(error) | GraphQLError::ResponseError(error) => {
                    if error
                        .status()
                        .is_some_and(|status| !matches!(status.as_u16(), 408 | 429 | 500..=599))
                    {
                        return false;
                    }
                    transient |= error.is_timeout() || error.is_connect() || error.is_body();
                }
                GraphQLError::StagingAccessBlocked | GraphQLError::IapChallengeBlocked => {
                    return false;
                }
            }
        }
        if let Some(error) = cause.downcast_ref::<reqwest::Error>() {
            if let Some(status) = error.status() {
                if !matches!(status.as_u16(), 408 | 429 | 500..=599) {
                    return false;
                }
                transient = true;
            }
            transient |= error.is_timeout() || error.is_connect() || error.is_body();
        }
        if let Some(error) = cause.downcast_ref::<HttpStatusError>() {
            if !matches!(error.status, 408 | 429 | 500..=599) {
                return false;
            }
            transient = true;
        }
    }
    transient
}

pub(crate) async fn refresh_restricted_credentials(
    task_id: &str,
    ai_client: &Arc<dyn AIClient>,
    use_factory_repositories: bool,
) -> Result<(), TaskGitCredentialsError> {
    let fetched = with_expiry(
        task_id,
        fetch(task_id, ai_client, true, use_factory_repositories),
    )
    .await
    .map_err(TaskGitCredentialsError::Request)?;
    match fetched {
        Ok(response) => apply_partial_response(task_id, response),
        Err(error) => apply_refresh(task_id, Err(error)),
    }
}

fn apply_partial_response(
    task_id: &str,
    response: RestrictedTaskGitCredentialsOutput,
) -> Result<(), TaskGitCredentialsError> {
    if response.failed_hosts.iter().any(|host| {
        !matches!(host.as_str(), GITHUB_HOST | "gitlab.com" | "dev.azure.com")
            || response
                .credentials
                .iter()
                .any(|credential| credential.host == *host)
    }) {
        return apply_refresh(
            task_id,
            Err(TaskGitCredentialsError::Request(anyhow::anyhow!(
                "Restricted credential response has unknown or contradictory failed forges"
            ))),
        );
    }
    let github_failed = response.failed_hosts.iter().any(|host| host == GITHUB_HOST);
    let partial = !response.failed_hosts.is_empty();
    let (github, other): (Vec<_>, Vec<_>) = response
        .credentials
        .into_iter()
        .partition(|credential| credential.host == GITHUB_HOST);
    let github_response = RestrictedTaskGitCredentialsOutput {
        github_credential_mode: response.github_credential_mode,
        credentials: github,
        failed_hosts: if github_failed {
            vec![GITHUB_HOST.into()]
        } else {
            Vec::new()
        },
    };
    let github_result = apply_refresh(
        task_id,
        validate_response(github_response, Utc::now(), Instant::now())
            .map_err(TaskGitCredentialsError::Request),
    );
    // Other forge failures must not revoke a successfully installed GitHub result.
    let other_result = (|| {
        let validated = validate_response(
            RestrictedTaskGitCredentialsOutput {
                github_credential_mode: response.github_credential_mode,
                credentials: other,
                failed_hosts: Vec::new(),
            },
            Utc::now(),
            Instant::now(),
        )?;
        if !validated.credentials.is_empty() {
            with_store(task_id, |store| {
                store.refresh_at = Some(validated.refresh_at);
                Ok(())
            })?;
        }
        write_git_credentials(&validated.credentials)
    })()
    .map_err(TaskGitCredentialsError::Request);
    github_result?;
    other_result?;
    if partial {
        return Err(TaskGitCredentialsError::Request(anyhow::anyhow!(
            "Git credential refresh was incomplete"
        )));
    }
    Ok(())
}

fn apply_refresh(
    task_id: &str,
    fetched: Result<ValidatedCredentials, TaskGitCredentialsError>,
) -> Result<(), TaskGitCredentialsError> {
    match fetched {
        Ok(response) => with_store(task_id, |store| apply(store, &response))
            .map_err(TaskGitCredentialsError::Request),
        Err(error) => {
            if let Err(cleanup) = credential_fetch_failed(
                task_id,
                is_transient_credential_error(&error),
                Instant::now(),
            ) {
                log::error!("Failed to clear rejected GitHub credentials: {cleanup:#}");
                return Err(TaskGitCredentialsError::Request(cleanup.context(error)));
            }
            Err(error)
        }
    }
}

pub(crate) fn credential_fetch_failed(task_id: &str, transient: bool, now: Instant) -> Result<()> {
    with_store(task_id, |store| {
        store.refresh_at = Some(now + REFRESH_FALLBACK_INTERVAL);
        if !transient || store.expires_at.is_none_or(|expiry| expiry <= now) {
            clear_store(store)?;
        }
        Ok(())
    })
}

fn clear_store(store: &mut ManagedStore) -> Result<()> {
    if let Err(write_error) = write_store(&store.directory, "") {
        let remove_result = (|| {
            let metadata = fs::symlink_metadata(&store.directory)?;
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "Unsafe managed GitHub credential path"
            );
            match fs::remove_file(store.directory.join("credentials")) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                Err(error) => Err(anyhow::Error::new(error)),
            }
        })();
        if let Err(remove_error) = remove_result {
            return Err(CredentialStoreCleanupError(remove_error.context(write_error)).into());
        }
    }
    store.expires_at = None;
    Ok(())
}

#[cfg(test)]
fn clear_restricted_credentials(task_id: &str) -> Result<()> {
    with_store(task_id, |store| {
        store.refresh_at = None;
        clear_store(store)
    })
}

pub(crate) fn schedule_refresh_retry(task_id: &str, now: Instant) -> Result<()> {
    with_store(task_id, |store| {
        store.refresh_at = Some(now + REFRESH_FALLBACK_INTERVAL);
        Ok(())
    })
}

pub(crate) fn refresh_delay(task_id: &str, now: Instant) -> Duration {
    with_store(task_id, |store| {
        Ok(store
            .refresh_at
            .map(|deadline| deadline.saturating_duration_since(now)))
    })
    .ok()
    .flatten()
    .unwrap_or(REFRESH_FALLBACK_INTERVAL)
}

fn expire_credentials(task_id: &str, now: Instant) -> Result<()> {
    with_store(task_id, |store| {
        if store.expires_at.is_some_and(|expiry| expiry <= now) {
            clear_store(store)?;
        }
        Ok(())
    })
}

pub(crate) async fn with_expiry<T>(task_id: &str, operation: impl Future<Output = T>) -> Result<T> {
    let operation = operation.fuse();
    futures::pin_mut!(operation);
    loop {
        let expiry = with_store(task_id, |store| Ok(store.expires_at))
            .ok()
            .flatten();
        let Some(expiry) = expiry else {
            return Ok(operation.await);
        };
        let timer = Timer::after(expiry.saturating_duration_since(Instant::now())).fuse();
        futures::pin_mut!(timer);
        futures::select_biased! {
            _ = timer => {
                expire_credentials(task_id, Instant::now())?;
            }
            result = operation => return Ok(result),
        }
    }
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
        clear_store(store)?;
        *managed = None;
    }
    Ok(())
}

pub(crate) fn active_restricted_child_environment() -> Result<HashMap<OsString, OsString>> {
    let task_id = MANAGED_STORE
        .lock()
        .map_err(|_| anyhow::anyhow!("GitHub store lock failed"))?
        .as_ref()
        .map(|store| store.task_id.clone());
    match task_id {
        Some(task_id) => restricted_child_environment(&task_id),
        None => Ok(HashMap::new()),
    }
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

pub(crate) fn protected_environment_key(key: &str) -> bool {
    GIT_OVERRIDE_ENV.contains(&key)
        || AUTH_ENV.contains(&key)
        || matches!(
            key,
            "HOME"
                | "GH_CONFIG_DIR"
                | "GIT_CONFIG_GLOBAL"
                | "GIT_CONFIG_COUNT"
                | "GIT_TERMINAL_PROMPT"
                | "WARP_GITHUB_CREDENTIAL_MODE"
        )
}

fn valid_config_count(value: Option<&OsStr>) -> bool {
    value.is_none_or(|value| value.is_empty() || value == "0")
}

pub(crate) fn validate_restricted_child_environment(
    env: &HashMap<OsString, OsString>,
) -> Result<()> {
    ensure!(
        !GIT_OVERRIDE_ENV.iter().any(|name| env
            .get(OsStr::new(name))
            .is_some_and(|value| !value.is_empty()))
            && valid_config_count(
                env.get(OsStr::new("GIT_CONFIG_COUNT"))
                    .map(OsString::as_os_str)
            ),
        "Restricted GitHub mode does not allow task-provided Git configuration overrides",
    );
    Ok(())
}

#[cfg(test)]
#[path = "restricted_tests.rs"]
mod tests;

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "restricted_lifecycle_tests.rs"]
mod lifecycle_tests;
