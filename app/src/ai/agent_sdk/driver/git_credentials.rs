/// Git credentials management for cloud agent sandboxes.
///
/// This module handles:
/// - Writing provider credentials to `~/.git-credentials`, plus GitHub
///   credentials to `~/.config/gh/hosts.yml`, without requiring environment
///   variables.
/// - One-time git configuration (`credential.helper store`, SSH→HTTPS URL
///   rewrites).
/// - Configuring the git user identity from the server-returned username/email.
/// - An async refresh loop that periodically fetches a fresh token from the
///   server and overwrites the credential files, keeping long-running agents
///   authenticated for their entire duration.
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

use anyhow::{Context, Result};
// Use the project's allowed Command wrapper (not std::process::Command, which is
// disallowed by clippy rules because it flashes a terminal window on Windows).
use command::blocking::Command as BlockingCommand;

use crate::server::server_api::ai::{AIClient, GitCredential};

/// How long to wait between credential refresh attempts (~50 minutes, staying
/// well ahead of the shortest-lived one-hour token expiry).
pub(crate) const GIT_CREDENTIALS_REFRESH_INTERVAL: Duration = Duration::from_secs(50 * 60);

const DEFAULT_GIT_NAME: &str = "Warp";
const DEFAULT_GIT_EMAIL: &str = "agent@warp.dev";
const GITHUB_HOST: &str = "github.com";
const GH_HOSTS_FILENAME: &str = "hosts.yml";
const GLAB_HOST: &str = "gitlab.com";
const GLAB_CONFIG_FILENAME: &str = "config.yml";

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))
}

/// Write `content` to `path` using owner-only (0600) permissions.
///
/// On Unix the file is created with mode 0600 so no other user can read the
/// credential material. On non-Unix platforms the function falls back to the
/// standard write, relying on OS default permissions.
fn write_secret_file(path: &std::path::Path, content: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("Failed to open {} for writing", path.display()))?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    Ok(())
}

fn git_credentials_line(cred: &GitCredential) -> String {
    let userinfo = match &cred.username {
        Some(username) => format!("{username}:{}", cred.token),
        None => format!("x-access-token:{}", cred.token),
    };
    format!("https://{}@{}", userinfo, cred.host)
}

/// Extract the host from a `~/.git-credentials` line, i.e. everything after
/// the last `@`. Returns `None` for a line with no userinfo separator, which
/// is left alone rather than guessed at.
fn host_of_credentials_line(line: &str) -> Option<&str> {
    line.rsplit_once('@').map(|(_, host)| host)
}

/// Merge fresh credentials into the existing `~/.git-credentials` content,
/// replacing the line for each refreshed host and preserving every other line.
///
/// Merging rather than rebuilding is what makes a partial refresh safe: a host
/// missing from `credentials` was not refreshed this cycle, not revoked, and
/// dropping its line would leave `git` unable to authenticate against a forge
/// whose CLI is still working from its own config.
fn merge_git_credentials_file_content(existing: &str, credentials: &[GitCredential]) -> String {
    let refreshed_hosts = credentials
        .iter()
        .map(|cred| cred.host.as_str())
        .collect::<Vec<_>>();

    let mut lines = Vec::new();
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let superseded = host_of_credentials_line(trimmed)
            .is_some_and(|host| refreshed_hosts.contains(&host));
        if !superseded {
            lines.push(trimmed.to_string());
        }
    }
    for cred in credentials {
        lines.push(git_credentials_line(cred));
    }

    let mut content = lines.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    content
}

/// Write `~/.git-credentials`, replacing the entry for each host in
/// `credentials` and leaving every other host's entry in place.
///
/// Each credential entry is formatted as:
/// - `https://{username}:{token}@{host}` when a username is present
/// - `https://x-access-token:{token}@{host}` for service-account tokens
///
/// The write is done atomically: a temporary file is written then renamed.
fn write_git_credentials_file(credentials: &[GitCredential]) -> Result<()> {
    if credentials.is_empty() {
        return Ok(());
    }

    let home = home_dir()?;
    let path = home.join(".git-credentials");
    let tmp_path = home.join(".git-credentials.tmp");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let content = merge_git_credentials_file_content(&existing, credentials);
    write_secret_file(&tmp_path, &content)?;
    std::fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "Failed to rename {} to {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    Ok(())
}

/// Write `~/.config/gh/hosts.yml` so the `gh` CLI is authenticated.
///
/// The YAML format is stable for `gh` v2+:
/// ```yaml
/// github.com:
///     oauth_token: TOKEN
///     git_protocol: https
///     user: USERNAME
/// ```
///
/// The write is atomic: a temporary file is written then renamed.
fn write_gh_hosts_yml(credentials: &[GitCredential], home: &std::path::Path) -> Result<()> {
    let github_credentials = credentials
        .iter()
        .filter(|credential| credential.host == GITHUB_HOST)
        .collect::<Vec<_>>();
    if github_credentials.is_empty() {
        return Ok(());
    }
    let gh_config_dir = home.join(".config").join("gh");
    std::fs::create_dir_all(&gh_config_dir)
        .with_context(|| format!("Failed to create {}", gh_config_dir.display()))?;
    let path = gh_config_dir.join(GH_HOSTS_FILENAME);
    let tmp_path = gh_config_dir.join(format!("{GH_HOSTS_FILENAME}.tmp"));

    let mut yaml = String::new();
    for cred in github_credentials {
        yaml.push_str(&format!("{}:\n", cred.host));
        yaml.push_str(&format!("    oauth_token: {}\n", cred.token));
        yaml.push_str("    git_protocol: https\n");
        if let Some(username) = &cred.username {
            yaml.push_str(&format!("    user: {username}\n"));
        }
    }

    write_secret_file(&tmp_path, &yaml)?;
    std::fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "Failed to rename {} to {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    Ok(())
}

/// Write `~/.config/glab-cli/config.yml` so the `glab` CLI is authenticated.
///
/// The YAML format for glab is:
/// ```yaml
/// hosts:
///     gitlab.com:
///         token: TOKEN
///         git_protocol: https
///         api_protocol: https
/// ```
///
/// The write is atomic: a temporary file is written then renamed.
fn write_glab_config(credentials: &[GitCredential], home: &std::path::Path) -> Result<()> {
    let gitlab_credentials = credentials
        .iter()
        .filter(|credential| credential.host == GLAB_HOST)
        .collect::<Vec<_>>();
    if gitlab_credentials.is_empty() {
        return Ok(());
    }
    let glab_config_dir = home.join(".config").join("glab-cli");
    std::fs::create_dir_all(&glab_config_dir)
        .with_context(|| format!("Failed to create {}", glab_config_dir.display()))?;
    let path = glab_config_dir.join(GLAB_CONFIG_FILENAME);
    let tmp_path = glab_config_dir.join(format!("{GLAB_CONFIG_FILENAME}.tmp"));

    let mut yaml = String::new();
    yaml.push_str("hosts:\n");
    for cred in gitlab_credentials {
        yaml.push_str(&format!("    {}:\n", cred.host));
        yaml.push_str(&format!("        token: {}\n", cred.token));
        yaml.push_str("        git_protocol: https\n");
        yaml.push_str("        api_protocol: https\n");
    }

    write_secret_file(&tmp_path, &yaml)?;
    std::fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "Failed to rename {} to {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    Ok(())
}

/// Formats non-sensitive metadata for verifying local credential injection.
///
/// `failed_hosts` are named alongside the fresh ones so a partial cycle is
/// legible in the log: the surprising state is not "this host is missing" but
/// "this host is running on a credential older than the others".
pub(crate) fn credential_diagnostics(credentials: &[GitCredential], failed_hosts: &[String]) -> String {
    let mut entries = credentials
        .iter()
        .map(|credential| {
            format!(
                "{}(refreshed, token_present={}, username_present={})",
                credential.host,
                !credential.token.is_empty(),
                credential.username.is_some()
            )
        })
        .collect::<Vec<_>>();
    entries.extend(
        failed_hosts
            .iter()
            .map(|host| format!("{host}(stale, refresh failed; existing credential kept)")),
    );
    entries.join(", ")
}

/// Write every credential store from `credentials`.
///
/// The three writes are independent: one store failing must not skip the
/// others, since each authenticates a different tool and a partial write is
/// strictly better than none. The first error is returned once all three have
/// been attempted.
pub(crate) fn write_git_credentials(credentials: &[GitCredential]) -> Result<()> {
    write_git_credentials_with_failures(credentials, &[])
}

pub(crate) fn write_git_credentials_with_failures(
    credentials: &[GitCredential],
    failed_hosts: &[String],
) -> Result<()> {
    if credentials.is_empty() {
        return Ok(());
    }
    let home = home_dir()?;
    let outcomes = [
        write_git_credentials_file(credentials),
        write_gh_hosts_yml(credentials, &home),
        write_glab_config(credentials, &home),
    ];
    let mut first_error = None;
    for outcome in outcomes {
        if let Err(e) = outcome {
            log::warn!("Failed to write a git credential store: {e:#}");
            first_error.get_or_insert(e);
        }
    }
    log::info!(
        "Wrote {} git credential(s) to the local credential store: {}",
        credentials.len(),
        credential_diagnostics(credentials, failed_hosts)
    );
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

pub(crate) fn configure_git_credentials(credentials: &[GitCredential]) -> Result<()> {
    if credentials.is_empty() {
        return Ok(());
    }
    setup_git_config(credentials);
    configure_git_identity(credentials);
    // Recorded here rather than at each clone, because the refresh loop
    // deliberately does not redo identity work: identity is set once at
    // startup and a later credential rotation does not change who the commits
    // are authored as.
    record_host_identities(credentials);
    write_git_credentials(credentials)
}

/// Run a git config command, logging a warning on failure rather than
/// propagating the error (git may not be installed in all sandboxes).
fn run_git_config(key: &str, value: &str) {
    match BlockingCommand::new("git")
        .args(["config", "--global", key, value])
        .output()
    {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            log::warn!(
                "git config --global {key} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => {
            log::warn!("Failed to run git config --global {key}: {e}");
        }
    }
}

/// Like [`run_git_config`] but passes `--add` so the new value is appended to
/// any existing values for `key` rather than replacing them.
fn run_git_config_add(key: &str, value: &str) {
    match BlockingCommand::new("git")
        .args(["config", "--global", "--add", key, value])
        .output()
    {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            log::warn!(
                "git config --global --add {key} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => {
            log::warn!("Failed to run git config --global --add {key}: {e}");
        }
    }
}

/// Run one-time git configuration that is set at startup and never needs to
/// be refreshed:
/// - `credential.helper store` so git reads `~/.git-credentials`
/// - SSH→HTTPS URL rewrites for each credential host, covering both the
///   scp-style (`git@{host}:`) and explicit-protocol (`ssh://git@{host}/`)
///   URL forms, so operations on either form use HTTPS credentials instead
///   of looking for an SSH key.
pub(crate) fn setup_git_config(credentials: &[GitCredential]) {
    run_git_config("credential.helper", "store");
    // Use --add for both forms per host so all values coexist as a
    // multi-value key rather than each entry overwriting the previous one.
    for cred in credentials {
        let host = &cred.host;
        run_git_config_add(
            &format!("url.https://{host}/.insteadOf"),
            &format!("ssh://git@{host}/"),
        );
        run_git_config_add(
            &format!("url.https://{host}/.insteadOf"),
            &format!("git@{host}:"),
        );
    }
}

/// Configure the global git user identity from the server-returned credential.
///
/// Uses the first credential's `username`/`email` fields, falling back to the
/// Warp defaults when either is absent (e.g. service-account principals).
///
/// This is the fallback for work outside any checkout, and for a repository
/// whose forge issued no credential. A repository whose forge did gets its own
/// identity written locally instead; see [`configure_repository_git_identity`].
pub(crate) fn configure_git_identity(credentials: &[GitCredential]) {
    let (name, email) = identity_of(credentials.first());
    run_git_config("user.name", &name);
    run_git_config("user.email", &email);
}

/// One forge's git author identity, without its credential.
#[derive(Clone)]
struct HostIdentity {
    host: String,
    name: String,
    email: String,
}

/// The author identity for each forge, captured when credentials are first
/// configured.
///
/// The clone path needs each forge's identity but runs several layers below
/// credential configuration, and threading the credential list down to it
/// would carry tokens through code that only needs a name and an email. Only
/// the non-secret fields are retained.
static HOST_IDENTITIES: RwLock<Vec<HostIdentity>> = RwLock::new(Vec::new());

fn record_host_identities(credentials: &[GitCredential]) {
    let identities = credentials
        .iter()
        .map(|c| HostIdentity {
            host: c.host.clone(),
            name: c.username.as_deref().unwrap_or(DEFAULT_GIT_NAME).to_string(),
            email: c.email.as_deref().unwrap_or(DEFAULT_GIT_EMAIL).to_string(),
        })
        .collect::<Vec<_>>();
    match HOST_IDENTITIES.write() {
        Ok(mut stored) => *stored = identities,
        Err(e) => log::warn!("Failed to record git author identities: {e}"),
    }
}

/// Resolve a credential's author identity, falling back to the Warp defaults
/// for the fields it does not carry (e.g. a service-account principal).
fn identity_of(credential: Option<&GitCredential>) -> (String, String) {
    match credential {
        Some(c) => (
            c.username.as_deref().unwrap_or(DEFAULT_GIT_NAME).to_string(),
            c.email.as_deref().unwrap_or(DEFAULT_GIT_EMAIL).to_string(),
        ),
        None => (DEFAULT_GIT_NAME.to_string(), DEFAULT_GIT_EMAIL.to_string()),
    }
}

/// Select the recorded identity for `host`, falling back to the primary
/// forge's identity when no credential was issued for it.
///
/// Falling back to the primary rather than to the Warp default keeps a
/// repository whose forge issued no credential behaving exactly as it does
/// today, rather than quietly downgrading its authorship.
fn select_host_identity<'a>(
    identities: &'a [HostIdentity],
    host: &str,
) -> Option<&'a HostIdentity> {
    identities
        .iter()
        .find(|identity| identity.host == host)
        .or_else(|| identities.first())
}

fn recorded_identity_for_host(host: &str) -> Option<(String, String)> {
    let stored = HOST_IDENTITIES.read().ok()?;
    let matched = select_host_identity(&stored, host)?;
    Some((matched.name.clone(), matched.email.clone()))
}

/// Write `user.name`/`user.email` into one repository's local git config,
/// selecting the identity of the forge that hosts it.
///
/// The server already sends an identity per credential; only the sandbox
/// collapsed them. Without this, a checkout spanning two forges would author
/// every commit as whichever identity happened to be first in the list — the
/// GitHub bot, since the server orders GitHub before GitLab.
///
/// Local config rather than a global `includeIf gitdir:` entry, so the
/// identity is attached to the repository itself and survives a re-clone, a
/// move, or a nested checkout.
pub(crate) fn configure_repository_git_identity(repository_dir: &std::path::Path, host: &str) {
    let Some((name, email)) = recorded_identity_for_host(host) else {
        return;
    };
    run_repository_git_config(repository_dir, "user.name", &name);
    run_repository_git_config(repository_dir, "user.email", &email);
}

/// Run `git -C <dir> config <key> <value>`, logging a warning on failure
/// rather than propagating: a missing identity degrades authorship, it does
/// not stop the run.
fn run_repository_git_config(repository_dir: &std::path::Path, key: &str, value: &str) {
    let dir = repository_dir.to_string_lossy();
    match BlockingCommand::new("git")
        .args(["-C", dir.as_ref(), "config", key, value])
        .output()
    {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            log::warn!(
                "git -C {} config {key} failed: {}",
                repository_dir.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => {
            log::warn!(
                "Failed to run git -C {} config {key}: {e}",
                repository_dir.display()
            );
        }
    }
}

/// Perform one git credentials refresh attempt.
///
/// Returns `Ok(true)` when every applicable forge refreshed, and `Ok(false)`
/// when some refreshed and others failed — the caller retries on the backoff
/// in that case, because only the failed hosts are still stale. Returns `Err`
/// when the workload-token issuance or the whole server call fails.
#[tracing::instrument(name = "git_credentials::try_refresh", skip_all, err, fields(
    tags.cloud_agent = true,
    task_id,
))]
async fn try_refresh(task_id: &str, ai_client: &Arc<dyn AIClient>) -> Result<bool> {
    let workload_token =
        warp_isolation_platform::issue_workload_token(Some(Duration::from_secs(5 * 60)))
            .await
            .context("Failed to issue workload token for git credentials refresh")?
            .token;

    let response = ai_client
        .get_task_git_credentials(task_id.to_string(), workload_token)
        .await
        .context("Failed to fetch git credentials from server")?;

    if response.credentials.is_empty() && response.failed_hosts.is_empty() {
        log::debug!("No git credentials returned during refresh; skipping file write");
        return Ok(true);
    }

    match write_git_credentials_with_failures(&response.credentials, &response.failed_hosts) {
        Err(e) => {
            log::warn!("Failed to write refreshed git credentials: {e:#}");
        }
        _ => {
            log::info!("Git credentials refreshed successfully");
        }
    }
    Ok(response.failed_hosts.is_empty())
}

/// Infinite async loop that refreshes git credentials every
/// [`GIT_CREDENTIALS_REFRESH_INTERVAL`].
///
/// On each iteration:
/// 1. Issue a short-lived workload token.
/// 2. Call `taskGitCredentials` to get a fresh token from the server.
/// 3. Overwrite `~/.git-credentials` and refresh GitHub credentials in
///    `~/.config/gh/hosts.yml`.
///
/// On transient failure, the refresh is retried up to three times with
/// exponential backoff (1 min, 2 min, 4 min), keeping all retries within the
/// ~10-minute buffer before the one-hour token expires. If all retries fail,
/// a warning is logged and the next refresh is scheduled after the normal
/// interval.
///
/// This future never resolves — it is designed to be raced with the harness
/// execution future via `futures::select!` and dropped when the harness
/// completes.
pub(crate) async fn refresh_loop(task_id: String, ai_client: Arc<dyn AIClient>) {
    loop {
        warpui::r#async::Timer::after(GIT_CREDENTIALS_REFRESH_INTERVAL).await;

        log::info!("Refreshing git credentials for task {task_id}");

        let backoff_delays = [
            Duration::from_secs(60),
            Duration::from_secs(2 * 60),
            Duration::from_secs(4 * 60),
        ];
        let mut attempt = 0usize;
        loop {
            match try_refresh(&task_id, &ai_client).await {
                Ok(true) => break,
                // Some forges refreshed and others did not. The server
                // reissues only what is still stale, so retrying costs nothing
                // for the hosts that already succeeded.
                Ok(false) if attempt < backoff_delays.len() => {
                    let delay = backoff_delays[attempt];
                    log::warn!(
                        "Git credentials refreshed for some forges but not others (attempt {}); \
                         retrying the remaining ones in {}s",
                        attempt + 1,
                        delay.as_secs()
                    );
                    warpui::r#async::Timer::after(delay).await;
                    attempt += 1;
                }
                Ok(false) => {
                    log::warn!(
                        "Git credentials still stale for some forges after {} attempts; \
                         those forges may lose access before the next refresh cycle",
                        attempt + 1
                    );
                    break;
                }
                Err(e) if attempt < backoff_delays.len() => {
                    let delay = backoff_delays[attempt];
                    log::warn!(
                        "Git credentials refresh failed (attempt {}): {e:#}; retrying in {}s",
                        attempt + 1,
                        delay.as_secs()
                    );
                    warpui::r#async::Timer::after(delay).await;
                    attempt += 1;
                }
                Err(e) => {
                    log::warn!(
                        "Git credentials refresh failed after {} attempts: {e:#}; \
                         credentials may expire before next refresh cycle",
                        attempt + 1
                    );
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "git_credentials_tests.rs"]
mod tests;
