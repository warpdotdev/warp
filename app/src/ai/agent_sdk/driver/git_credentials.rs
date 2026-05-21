/// Git credentials management for cloud agent sandboxes.
///
/// This module handles:
/// - Writing `~/.git-credentials` and `~/.config/gh/hosts.yaml` so that `git`
///   and the `gh` CLI can authenticate to GitHub without requiring environment
///   variables.
/// - One-time git configuration (`credential.helper store`, SSH→HTTPS URL
///   rewrites).
/// - Configuring the git user identity from the server-returned username/email.
/// - An async refresh loop that periodically fetches a fresh token from the
///   server and overwrites the credential files, keeping long-running agents
///   authenticated for their entire duration.
use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};

use crate::server::server_api::ai::{AIClient, GitCredential};

// Use the project's allowed Command wrapper (not std::process::Command, which is
// disallowed by clippy rules because it flashes a terminal window on Windows).
use command::blocking::Command as BlockingCommand;

/// How long to wait between credential refresh attempts (~50 minutes, staying
/// well ahead of the one-hour GitHub token expiry).
pub(crate) const GIT_CREDENTIALS_REFRESH_INTERVAL: Duration = Duration::from_secs(50 * 60);

const DEFAULT_GIT_NAME: &str = "Oz";
const DEFAULT_GIT_EMAIL: &str = "oz-agent@warp.dev";

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
        use std::os::unix::fs::OpenOptionsExt as _;
        use std::os::unix::fs::PermissionsExt as _;
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

/// Write `~/.git-credentials` with the given credentials.
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

    let mut content = String::new();
    for cred in credentials {
        let userinfo = match &cred.username {
            Some(username) => format!("{username}:{}", cred.token),
            None => format!("x-access-token:{}", cred.token),
        };
        content.push_str(&format!("https://{}@{}\n", userinfo, cred.host));
    }

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

/// Write `~/.config/gh/hosts.yaml` so the `gh` CLI is authenticated.
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
fn write_gh_hosts_yaml(credentials: &[GitCredential]) -> Result<()> {
    if credentials.is_empty() {
        return Ok(());
    }

    let home = home_dir()?;
    let gh_config_dir = home.join(".config").join("gh");
    std::fs::create_dir_all(&gh_config_dir)
        .with_context(|| format!("Failed to create {}", gh_config_dir.display()))?;

    let path = gh_config_dir.join("hosts.yaml");
    let tmp_path = gh_config_dir.join("hosts.yaml.tmp");

    let mut yaml = String::new();
    for cred in credentials {
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

pub(crate) fn write_git_credentials(credentials: &[GitCredential]) -> Result<()> {
    write_git_credentials_file(credentials)?;
    write_gh_hosts_yaml(credentials)?;
    Ok(())
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

/// Configure the git user identity from the server-returned credential.
///
/// Uses the first credential's `username`/`email` fields, falling back to the
/// Oz defaults when either is absent (e.g. service-account principals).
pub(crate) fn configure_git_identity(credentials: &[GitCredential]) {
    let (name, email) = credentials
        .first()
        .map(|c| {
            (
                c.username.as_deref().unwrap_or(DEFAULT_GIT_NAME),
                c.email.as_deref().unwrap_or(DEFAULT_GIT_EMAIL),
            )
        })
        .unwrap_or((DEFAULT_GIT_NAME, DEFAULT_GIT_EMAIL));

    run_git_config("user.name", name);
    run_git_config("user.email", email);
}

/// Perform one git credentials refresh attempt.
///
/// Returns `Ok(())` on success (including when the server returns no
/// credentials). Returns `Err` when the workload-token issuance or the server
/// API call fails — these are transient failures worth retrying.
async fn try_refresh(task_id: &str, ai_client: &Arc<dyn AIClient>) -> Result<()> {
    let workload_token =
        warp_isolation_platform::issue_workload_token(Some(Duration::from_secs(5 * 60)))
            .await
            .context("Failed to issue workload token for git credentials refresh")?
            .token;

    let credentials = ai_client
        .get_task_git_credentials(task_id.to_string(), workload_token)
        .await
        .context("Failed to fetch git credentials from server")?;

    if credentials.is_empty() {
        log::debug!("No git credentials returned during refresh; skipping file write");
        return Ok(());
    }

    if let Err(e) = write_git_credentials(&credentials) {
        log::warn!("Failed to write refreshed git credentials: {e:#}");
    } else {
        log::info!("Git credentials refreshed successfully");
    }
    Ok(())
}

/// Infinite async loop that refreshes git credentials every
/// [`GIT_CREDENTIALS_REFRESH_INTERVAL`].
///
/// On each iteration:
/// 1. Issue a short-lived workload token.
/// 2. Call `taskGitCredentials` to get a fresh token from the server.
/// 3. Overwrite `~/.git-credentials` and `~/.config/gh/hosts.yaml`.
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
                Ok(()) => break,
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
