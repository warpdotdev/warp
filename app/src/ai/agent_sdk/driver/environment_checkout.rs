use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{fmt, io};

use anyhow::anyhow;
use async_compat::Compat;
use build_cache::metadata::CacheUsage;
use build_cache::{RepoCacheKey, RepoIdentity};
use cloud_object_models::{CodeForge, SourceRepo};
use command::Stdio;
use command::r#async::Command;
use futures::{AsyncWriteExt as _, StreamExt as _, stream};
use instant::Instant;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tracing::Instrument as _;
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef};
use warp_cli::environment_checkout::EnvironmentCheckoutArgs;
use warp_core::features::FeatureFlag;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::cache_setup;
use super::environment_checkout_protocol::{
    CheckoutBatch, CheckoutFailureKind, CheckoutOutcome, CheckoutReport, CheckoutRequest,
    CloneFailureCredentialIdentity, CloneFailureIdentityDiagnostics, parse_resolved_head_sha,
    sanitize_git_author_name, sanitize_git_credential_username,
};
use super::failure_output;

const CHECKOUT_WORKERS: usize = 4;
const CAPTURE_BYTES: usize = 64 * 1024;
const OUTPUT_TRUNCATION_MARKER: &str = "\n… git output truncated …\n";
const MIRRORS_DIRECTORY: &str = "git-mirrors";
const IDENTITY_QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const HEAD_CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

/// Runs the environment checkout command to completion.
pub(crate) fn run(args: &EnvironmentCheckoutArgs) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| sanitized_error("could not start checkout runtime", error))?;
    let span = tracing::info_span!(
        "environment_checkout",
        tags.cloud_agent = true,
        remove_origins_only = args.remove_origins_only,
        result_ok = tracing::field::Empty,
    );
    let result = runtime.block_on(run_async(args).instrument(span.clone()));
    span.record("result_ok", result.is_ok());
    result
}

/// Returns a redacted label naming the request's repository and checkout directory for logs.
fn repository_label(request: &CheckoutRequest) -> String {
    let repo = source_repo(request);
    failure_output::prepare_failure_output(
        &format!(
            "{}/{}/{} (checkout {})",
            repo.code_forge.unwrap_or_default().host(),
            repo.owner,
            repo.repo,
            request.checkout_name
        ),
        OUTPUT_TRUNCATION_MARKER,
    )
}

/// Removes the `origin` remote configuration from an existing checkout, leaving its refs intact.
async fn remove_origin(target: &Path, git: &mut Git) -> Result<(), ()> {
    if !fs::symlink_metadata(target)
        .await
        .map_err(|error| git.record_error("could not inspect origin cleanup target", error))?
        .is_dir()
    {
        git.record("origin cleanup target is not a repository directory");
        return Err(());
    }

    let remotes = git.run("remote list", target, &["remote"]).await?;
    if remotes.lines().any(|remote| remote == "origin") {
        // Removing the remote itself can collide on case-insensitive tracking-ref lock paths.
        git.run(
            "origin remote removal",
            target,
            &["config", "--local", "--remove-section", "remote.origin"],
        )
        .await?;
    }
    Ok(())
}

/// Performs the checkout batch described by `args`, writing one report describing the state,
/// timing, diagnostics and - for successful checkouts - the resolved `HEAD` of every request.
async fn run_async(args: &EnvironmentCheckoutArgs) -> anyhow::Result<()> {
    let bytes = fs::read(&args.requests_file)
        .await
        .map_err(|error| sanitized_error("could not read environment checkout requests", error))?;
    let batch = CheckoutBatch::parse(&bytes)
        .map_err(|error| sanitized_error("could not parse environment checkout requests", error))?;
    let mirror_root = (!args.remove_origins_only)
        .then(optional_mirror_root)
        .flatten();
    let mut outcomes = checkout_batch(
        &batch,
        mirror_root.as_deref(),
        args.remove_origins_only,
        args.fail_if_target_exists,
    )
    .await?;
    let failed = outcomes.iter().any(|outcome| outcome.failure.is_some());
    let identity_diagnostics = if failed && !args.remove_origins_only {
        Some(collect_failure_identity(&batch, &outcomes).await)
    } else {
        None
    };
    if !failed && !args.remove_origins_only {
        resolve_heads(&batch, &mut outcomes).await;
    }
    let bytes = serde_json::to_vec(&CheckoutReport {
        outcomes,
        identity_diagnostics,
    })?;
    fs::write(&args.report_file, bytes)
        .await
        .map_err(|error| sanitized_error("could not write environment checkout report", error))?;
    if failed {
        Err(anyhow!("structured repository checkout failed"))
    } else {
        Ok(())
    }
}

fn sanitized_error(context: &str, error: impl fmt::Display) -> anyhow::Error {
    // JSON errors can echo untrusted values, so do not retain an unredacted error source.
    anyhow!(
        "{}",
        failure_output::prepare_failure_output(
            &format!("{context}: {error:#}"),
            OUTPUT_TRUNCATION_MARKER
        )
    )
}

/// Resolves missing checkout HEADs with bounded concurrency and a per-checkout timeout.
async fn resolve_heads(batch: &CheckoutBatch, outcomes: &mut [CheckoutOutcome]) {
    let mut capture = stream::iter(
        outcomes
            .iter_mut()
            .filter(|outcome| outcome.resolved_head.is_none())
            .map(|outcome| async move {
                let Some(request) = batch.repositories.get(outcome.request_index) else {
                    return;
                };
                let mut git = Git::default();
                let head = tokio::time::timeout(
                    HEAD_CAPTURE_TIMEOUT,
                    git.run(
                        "HEAD resolution",
                        &batch.working_dir.join(&request.checkout_name),
                        &["rev-parse", "--verify", "HEAD"],
                    ),
                )
                .await;
                let Ok(head) = head else {
                    log::warn!(
                        "Repository {}: timed out capturing resolved HEAD",
                        repository_label(request)
                    );
                    return;
                };
                outcome.resolved_head = head.ok().and_then(|head| parse_resolved_head_sha(&head));
                match &outcome.resolved_head {
                    Some(head) => log::info!(
                        "Repository {}: resolved HEAD {head}",
                        repository_label(request)
                    ),
                    None => log::warn!(
                        "Repository {}: could not resolve HEAD\n{}",
                        repository_label(request),
                        git.diagnostics
                    ),
                }
            }),
    )
    .buffer_unordered(CHECKOUT_WORKERS);
    while capture.next().await.is_some() {}
}

/// Runs `git` with an optional stdin `input` and returns its stdout, or `None` if it fails,
/// produces truncated output, or exceeds `IDENTITY_QUERY_TIMEOUT`.
#[tracing::instrument(skip_all, fields(tags.cloud_agent = true))]
async fn identity_query(cwd: &Path, args: &[&str], input: Option<&str>) -> Option<String> {
    tokio::time::timeout(IDENTITY_QUERY_TIMEOUT, async {
        let mut child = Command::new("git")
            .current_dir(cwd)
            .env_remove("WARP_CLOUD_AGENT_OTLP_TOKEN")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        if let Some(input) = input {
            let mut stdin = child.stdin.take()?;
            stdin.write_all(input.as_bytes()).await.ok()?;
            // Dropping Windows subprocess stdin without flushing can discard the buffered query.
            stdin.flush().await.ok()?;
        }
        let stdout = child.stdout.take()?;
        let (output, status) = tokio::join!(capture(Compat::new(stdout)), child.status());
        let (stdout, truncated) = output.ok()?;
        (status.ok()?.success() && !truncated).then_some(stdout)
    })
    .await
    .ok()
    .flatten()
}

/// Extracts the sanitized username from `git credential fill` output, if it has exactly one.
fn git_credential_username(stdout: &str) -> Option<String> {
    let mut usernames = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("username="));
    let username = usernames.next()?;
    if usernames.next().is_some() {
        return None;
    }
    sanitize_git_credential_username(username)
}

/// Collects the git author and the credential username per forge host, to help diagnose failed
/// checkouts.
async fn collect_failure_identity(
    batch: &CheckoutBatch,
    outcomes: &[CheckoutOutcome],
) -> CloneFailureIdentityDiagnostics {
    let mut seen = HashSet::new();
    let hosts = outcomes
        .iter()
        .filter(|outcome| outcome.failure.is_some())
        .filter_map(|outcome| batch.repositories.get(outcome.request_index))
        .map(|request| source_repo(request).code_forge.unwrap_or_default().host())
        .filter(|host| seen.insert(*host))
        .collect::<Vec<_>>();
    let credentials = futures::future::join_all(hosts.into_iter().map(|host| async move {
        let input = format!("protocol=https\nhost={host}\n\n");
        let username = identity_query(&batch.working_dir, &["credential", "fill"], Some(&input))
            .await
            .and_then(|stdout| git_credential_username(&stdout));
        CloneFailureCredentialIdentity {
            host: host.to_owned(),
            username,
        }
    }));
    let author = identity_query(&batch.working_dir, &["config", "--get", "user.name"], None);
    let (author, credentials) = tokio::join!(author, credentials);
    CloneFailureIdentityDiagnostics {
        author: author.and_then(|value| sanitize_git_author_name(&value)),
        credentials,
    }
}

/// Reports the git mirror directory as cache usage when it exists under `cache_root`.
pub(super) fn mirror_cache_usage(cache_root: &Path) -> Vec<CacheUsage> {
    if std::fs::symlink_metadata(cache_root.join(MIRRORS_DIRECTORY))
        .is_ok_and(|metadata| metadata.is_dir())
    {
        vec![CacheUsage {
            path: MIRRORS_DIRECTORY.into(),
            cache_framework: Some("git".to_owned()),
            mount_target: Vec::new(),
        }]
    } else {
        Vec::new()
    }
}

/// Returns the canonical, writable directory that holds the git mirrors, or `None` when mirror
/// caching is disabled or the directory is unusable.
fn optional_mirror_root() -> Option<PathBuf> {
    if !FeatureFlag::GitMirrorCache.is_enabled() {
        return None;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let cache_root = cache_setup::enabled_cache_root()?;
        let root = cache_root.join(MIRRORS_DIRECTORY);
        if std::fs::symlink_metadata(&root).is_ok_and(|metadata| !metadata.is_dir()) {
            log::warn!("Git mirror cache unavailable: mirror root is not a directory");
            return None;
        }
        let prepare = || -> io::Result<PathBuf> {
            std::fs::create_dir_all(&root)?;
            let _writability_probe = tempfile::NamedTempFile::new_in(&root)?;
            std::fs::canonicalize(&root)
        };
        match prepare() {
            Ok(root) => Some(root),
            Err(error) => {
                log::warn!("{}", sanitized_error("Git mirror cache unavailable", error));
                None
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Converts the request's source identity into a [`SourceRepo`].
fn source_repo(request: &CheckoutRequest) -> SourceRepo {
    let forge = match request.source.code_forge {
        RepositoryForge::GitHub => CodeForge::GitHub,
        RepositoryForge::GitLab => CodeForge::GitLab,
        RepositoryForge::AzureDevOps => CodeForge::AzureDevOps,
    };
    SourceRepo::new(
        forge,
        request.source.repo_owner.clone(),
        request.source.repo_name.clone(),
    )
}

/// Returns the cache key that identifies the mirror for the request's source repository.
fn mirror_key(request: &CheckoutRequest) -> RepoCacheKey {
    let repo = source_repo(request);
    RepoCacheKey::derive(&RepoIdentity::new(
        repo.code_forge.unwrap_or_default().host(),
        repo.owner,
        repo.repo,
    ))
}

/// Runs every request in the batch, returning one outcome per request in order. Requests that
/// share a mirror run one after another, and at most `CHECKOUT_WORKERS` groups run concurrently.
#[tracing::instrument(skip_all, fields(
    tags.cloud_agent = true,
    repository_count = batch.repositories.len(),
    cache_enabled = mirror_root.is_some(),
    remove_origins_only,
))]
async fn checkout_batch(
    batch: &CheckoutBatch,
    mirror_root: Option<&Path>,
    remove_origins_only: bool,
    fail_if_target_exists: bool,
) -> anyhow::Result<Vec<CheckoutOutcome>> {
    batch.validate().map_err(|error| anyhow!(error))?;
    let mut groups = HashMap::<_, Vec<_>>::new();
    for (index, request) in batch.repositories.iter().enumerate() {
        groups
            .entry(mirror_key(request))
            .or_default()
            .push((index, request));
    }
    let mut outcomes = vec![None; batch.repositories.len()];
    let mut completed = stream::iter(groups.into_values().map(|requests| async move {
        let mut outcomes = Vec::new();
        for (index, request) in requests {
            let label = repository_label(request);
            log::info!("Repository {label}: starting checkout");
            let started = Instant::now();
            let mut git = Git::default();
            let span = tracing::info_span!(
                "repository_checkout",
                tags.cloud_agent = true,
                request_index = index,
                fetch_branch_only = request.fetch_branch_only,
                result_ok = tracing::field::Empty,
                failure = tracing::field::Empty,
            );
            let result = async {
                if remove_origins_only {
                    remove_origin(&batch.working_dir.join(&request.checkout_name), &mut git)
                        .await
                        .map_err(|_| CheckoutFailureKind::RemoveOrigin)
                } else {
                    checkout(
                        request,
                        &batch.working_dir,
                        mirror_root,
                        fail_if_target_exists,
                        &mut git,
                    )
                    .await
                }
            }
            .instrument(span.clone())
            .await;
            span.record("result_ok", result.is_ok());
            if let Err(failure) = &result {
                span.record("failure", tracing::field::debug(failure));
            }
            drop(span);
            let duration = started.elapsed();
            log::info!(
                "Repository {label}: {} after {duration:.1?}",
                if result.is_ok() { "finished" } else { "failed" }
            );
            outcomes.push(CheckoutOutcome {
                request_index: index,
                failure: result.err(),
                diagnostics: git.diagnostics,
                duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
                resolved_head: None,
            });
        }
        outcomes
    }))
    .buffer_unordered(CHECKOUT_WORKERS);
    while let Some(group) = completed.next().await {
        for outcome in group {
            let index = outcome.request_index;
            outcomes[index] = Some(outcome);
        }
    }
    let outcomes = outcomes
        .into_iter()
        .map(|outcome| outcome.expect("checkout worker outcome"))
        .collect::<Vec<_>>();
    for outcome in &outcomes {
        if outcome.failure.is_some() {
            log::error!(
                "Repository {} diagnostics:\n{}",
                repository_label(&batch.repositories[outcome.request_index]),
                outcome.diagnostics
            );
        }
    }
    Ok(outcomes)
}

/// Git CLI wrapper that buffers diagnostic messages. Every request reports them in the checkout
/// report, so they are redacted and bounded in size.
#[derive(Default)]
struct Git {
    diagnostics: String,
}

impl Git {
    /// Runs `git` in `cwd` and returns its stdout on success. What git reports, and any failure,
    /// is kept in the diagnostics for later reporting, described by `operation`.
    async fn run(&mut self, operation: &str, cwd: &Path, args: &[&str]) -> Result<String, ()> {
        self.run_with_env(operation, cwd, args, &[]).await
    }

    /// Like [`Git::run`], with additional environment variables set for the process.
    #[tracing::instrument(skip_all, fields(
        tags.cloud_agent = true,
        operation,
        result_ok = false,
    ))]
    async fn run_with_env(
        &mut self,
        operation: &str,
        cwd: &Path,
        args: &[&str],
        env: &[(&str, &OsStr)],
    ) -> Result<String, ()> {
        let child = Command::new("git")
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .envs(env.iter().copied())
            .env_remove("WARP_CLOUD_AGENT_OTLP_TOKEN")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let mut child = child.map_err(|error| {
            self.record_error(&format!("could not start Git {operation}"), error)
        })?;
        let stdout = child.stdout.take().expect("Git stdout pipe");
        let stderr = child.stderr.take().expect("Git stderr pipe");
        let (stdout, stderr, status) = tokio::join!(
            capture(Compat::new(stdout)),
            capture(Compat::new(stderr)),
            child.status(),
        );
        let stdout = stdout.map_err(|error| {
            self.record_error(&format!("could not read Git {operation} stdout"), error)
        });
        let stderr = stderr.map_err(|error| {
            self.record_error(&format!("could not read Git {operation} stderr"), error)
        });
        let status = status.map_err(|error| {
            self.record_error(&format!("could not wait for Git {operation}"), error)
        });
        if let Ok((stderr, stderr_truncated)) = &stderr {
            if *stderr_truncated {
                self.record(OUTPUT_TRUNCATION_MARKER);
            }
            self.record(stderr);
        }
        let (Ok((stdout, truncated)), Ok(_), Ok(status)) = (stdout, stderr, status) else {
            return Err(());
        };
        if status.success() && !truncated {
            tracing::Span::current().record("result_ok", true);
            Ok(stdout)
        } else {
            self.record(&stdout);
            if !status.success() {
                self.record_error(&format!("Git {operation} failed"), status);
            }
            if truncated {
                self.record(&format!(
                    "Git {operation} stdout exceeded the capture limit"
                ));
            }
            Err(())
        }
    }

    /// Appends `output` to the diagnostics, keeping them redacted and bounded in size.
    fn record(&mut self, output: &str) {
        let combined = format!("{}\n{output}", self.diagnostics);
        self.diagnostics =
            failure_output::prepare_failure_output(&combined, OUTPUT_TRUNCATION_MARKER);
    }

    fn record_error(&mut self, context: &str, error: impl fmt::Display) {
        self.record(&format!("{context}: {error:#}"));
    }
}

/// Reads `reader` to the end, retaining only the tail of long output. The flag reports whether
/// any output was dropped.
async fn capture(mut reader: impl AsyncRead + Unpin) -> io::Result<(String, bool)> {
    let mut bytes = VecDeque::new();
    let mut buffer = [0; 8192];
    let mut truncated = false;
    let mut starts_on_line = true;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        bytes.extend(&buffer[..count]);
        if bytes.len() > CAPTURE_BYTES {
            let excess = bytes.len() - CAPTURE_BYTES;
            starts_on_line = bytes[excess - 1] == b'\n';
            bytes.drain(..excess);
            truncated = true;
        }
    }
    let bytes = bytes.make_contiguous();
    let start = if truncated && !starts_on_line {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |index| index + 1)
    } else {
        0
    };
    Ok((
        String::from_utf8_lossy(&bytes[start..]).into_owned(),
        truncated,
    ))
}

/// Checks out one request into `working_dir`, refusing existing targets when requested.
/// New checkouts use the mirror when available, falling back to the network if that fails.
async fn checkout(
    request: &CheckoutRequest,
    working_dir: &Path,
    mirror_root: Option<&Path>,
    fail_if_target_exists: bool,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    let target = working_dir.join(&request.checkout_name);
    let existing = match fs::symlink_metadata(&target).await {
        Ok(_) if fail_if_target_exists => {
            git.record("checkout target already exists; choose an unused checkout name");
            return Err(CheckoutFailureKind::Clone);
        }
        Ok(metadata) if metadata.is_dir() => true,
        Ok(_) => {
            git.record("checkout target is not a repository directory");
            return Err(CheckoutFailureKind::Clone);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            git.record_error("could not inspect checkout target", error);
            return Err(CheckoutFailureKind::Clone);
        }
    };
    let url = source_repo(request).https_clone_url();
    if existing {
        return checkout_existing(request, &url, &target, git).await;
    }
    if let Some(root) = mirror_root {
        let mirror = root.join(mirror_key(request).as_str());
        if attempt_cached_checkout(request, &url, &target, &mirror, git).await?
            == CachedCheckout::Built
        {
            return Ok(());
        }
        let label = repository_label(request);
        log::info!("Repository {label}: unable to use cache, falling back to direct clone");
        if !git.diagnostics.is_empty() {
            log::info!(
                "Repository {label}: cache failure diagnostics:\n{}",
                git.diagnostics
            );
        }
        git.diagnostics.clear();
    }
    fs::create_dir(&target).await.map_err(|error| {
        git.record_error("could not create checkout target", error);
        CheckoutFailureKind::Clone
    })?;
    checkout_direct(request, &url, &target, git).await
}

/// Result of trying to build a checkout from the mirror.
#[derive(Debug, PartialEq, Eq)]
enum CachedCheckout {
    Built,
    /// The attempt did not produce a checkout, and no checkout it created is left behind.
    Failed,
}

/// Refreshes the mirror and builds the checkout at `target` from it. Only a checkout that this
/// attempt created is removed when it fails, so a `target` that already exists is never touched.
/// Fails outright if a failed checkout cannot be removed, since the target is then unusable.
async fn attempt_cached_checkout(
    request: &CheckoutRequest,
    url: &str,
    target: &Path,
    mirror: &Path,
    git: &mut Git,
) -> Result<CachedCheckout, CheckoutFailureKind> {
    let label = repository_label(request);
    let Ok(default) = refresh_mirror(mirror, url, &label, git).await else {
        return Ok(CachedCheckout::Failed);
    };
    if let Err(error) = fs::create_dir(target).await {
        git.record_error("could not create cached checkout target", error);
        return Ok(CachedCheckout::Failed);
    }
    if checkout_cached(request, url, target, mirror, &default, git)
        .await
        .is_ok()
    {
        return Ok(CachedCheckout::Built);
    }
    fs::remove_dir_all(target).await.map_err(|error| {
        git.record_error("could not remove failed cached checkout target", error);
        CheckoutFailureKind::Clone
    })?;
    Ok(CachedCheckout::Failed)
}

/// Ensures `mirror` is a valid, up-to-date bare copy of `url`, rebuilding it if it is not or if it
/// cannot be refreshed. Returns the remote's default branch, which the mirror's `HEAD` follows.
///
/// The mirror holds every ref the remote advertises, including ones such as pull request refs
/// that no branch reaches, so checkouts can resolve any commit the remote can serve from it. It is
/// always a complete copy because checkouts borrow its objects. Its configuration is rewritten on
/// every use so that nothing left over from earlier runs, such as a URL carrying credentials or
/// unrelated settings, is trusted.
async fn refresh_mirror(
    mirror: &Path,
    url: &str,
    label: &str,
    git: &mut Git,
) -> Result<String, ()> {
    let parent = mirror
        .parent()
        .ok_or_else(|| git.record("cache mirror has no parent directory"))?;
    let existing = fs::symlink_metadata(mirror).await;
    let config_path = mirror.join("config");
    let config_path_str = config_path
        .to_str()
        .ok_or_else(|| git.record("cache mirror config path is not valid UTF-8"))?;
    let canonical_config = format!(
        "[core]\n\trepositoryformatversion = 0\n\tbare = true\n\
         [remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/*:refs/*\n\tmirror = true\n"
    );
    // A mirror configured as partial would lack objects that checkouts expect to borrow from it.
    let mut valid = existing.as_ref().is_ok_and(|metadata| metadata.is_dir())
        && git
            .run(
                "cache mirror config read",
                parent,
                &[
                    "config",
                    "--file",
                    config_path_str,
                    "--no-includes",
                    "--list",
                ],
            )
            .await
            .is_ok_and(|config| {
                config.lines().all(|line| {
                    let name = line
                        .split('=')
                        .next()
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    !name.ends_with(".promisor")
                        && !name.ends_with(".partialclonefilter")
                        && name != "extensions.partialclone"
                })
            });
    if valid {
        fs::write(&config_path, &canonical_config)
            .await
            .map_err(|error| git.record_error("could not write cache mirror config", error))?;
        valid = git
            .run(
                "cache mirror bare check",
                mirror,
                &["rev-parse", "--is-bare-repository"],
            )
            .await
            .is_ok_and(|value| value.trim() == "true");
    }
    let mut cloned = false;
    if valid {
        log::info!("Repository {label}: found valid cache");
    } else {
        match &existing {
            Ok(_) => log::info!("Repository {label}: invalid cache; rebuilding"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                log::info!("Repository {label}: preparing cache");
            }
            Err(error) => {
                git.record_error("could not inspect cache mirror", error);
                return Err(());
            }
        }
        install_mirror(mirror, url, git).await?;
        cloned = true;
    }
    if !cloned {
        // `--prune` and `--prune-tags` drop refs deleted upstream so removed history is not
        // served from the mirror. Protocol v2 keeps the ref advertisement small for repositories
        // with many refs, and submodules are irrelevant to a bare mirror.
        let refreshed = git
            .run(
                "cache mirror fetch",
                mirror,
                &[
                    "-c",
                    "protocol.version=2",
                    "fetch",
                    "--no-recurse-submodules",
                    "--prune",
                    "--prune-tags",
                    "origin",
                ],
            )
            .await
            .is_ok();
        if !refreshed {
            // A fetch can fail where a fresh clone succeeds, such as on stale lock files left by
            // an interrupted run, or on branches that differ only in case on a case-insensitive
            // filesystem.
            log::info!("Repository {label}: cache refresh failed; rebuilding");
            install_mirror(mirror, url, git).await?;
        }
    }
    // Fetching never moves the mirror's `HEAD`, so follow the remote's default branch explicitly.
    let head = remote_default_branch(mirror, git).await?;
    git.run(
        "cache mirror HEAD update",
        mirror,
        &["symbolic-ref", "HEAD", &format!("refs/heads/{head}")],
    )
    .await?;
    Ok(head)
}

/// Puts a fresh `git clone --mirror` of `url` at `mirror`, replacing whatever is there. The clone
/// is made beside the mirror and moved into place only once it is complete, so a failure or an
/// interrupted run never leaves a partial mirror at `mirror`, and an existing mirror stays in
/// place until the replacement is ready.
///
/// `--mirror` makes a bare repository whose `+refs/*:refs/*` refspec lets later fetches update
/// and prune every ref, not only branches. A clone also records every ref in `packed-refs` in
/// one step, so unlike a fetch it succeeds for branches that differ only in case on a
/// case-insensitive filesystem.
async fn install_mirror(mirror: &Path, url: &str, git: &mut Git) -> Result<(), ()> {
    let parent = mirror
        .parent()
        .ok_or_else(|| git.record("cache mirror has no parent directory"))?;
    let mut staging_name = mirror
        .file_name()
        .ok_or_else(|| git.record("cache mirror has no name"))?
        .to_owned();
    staging_name.push(".staging");
    let staging = parent.join(staging_name);
    let staging_path = staging
        .to_str()
        .ok_or_else(|| git.record("cache mirror staging path is not valid UTF-8"))?;
    // Anything here is left over from an interrupted run.
    let _ = fs::remove_dir_all(&staging).await;
    let cloned = git
        .run(
            "cache mirror clone",
            parent,
            &["clone", "--mirror", "--quiet", "--", url, staging_path],
        )
        .await;
    if cloned.is_err() {
        let _ = fs::remove_dir_all(&staging).await;
        return Err(());
    }
    match fs::symlink_metadata(mirror).await {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(mirror).await,
        Ok(_) => fs::remove_file(mirror).await,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
    .map_err(|error| git.record_error("could not remove stale cache mirror", error))?;
    fs::rename(&staging, mirror)
        .await
        .map_err(|error| git.record_error("could not move cache mirror into place", error))
}

/// Returns the validated name of the branch that the `origin` remote's `HEAD` points to.
async fn remote_default_branch(target: &Path, git: &mut Git) -> Result<String, ()> {
    let output = git
        .run(
            "default branch lookup",
            target,
            &["ls-remote", "--symref", "origin", "HEAD"],
        )
        .await?;
    let branch = output
        .lines()
        .find_map(|line| {
            line.strip_prefix("ref: refs/heads/")
                .and_then(|value| value.strip_suffix("\tHEAD"))
        })
        .ok_or_else(|| git.record("git ls-remote did not report a symbolic default branch"))?;
    git.run(
        "default branch validation",
        target,
        &["check-ref-format", "--branch", branch],
    )
    .await?;
    Ok(branch.to_owned())
}

/// Moves a repository that is already present to the requested head.
async fn checkout_existing(
    request: &CheckoutRequest,
    url: &str,
    target: &Path,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    if request.fetch_branch_only {
        let action = if git
            .run(
                "origin remote lookup",
                target,
                &["remote", "get-url", "origin"],
            )
            .await
            .is_ok()
        {
            "set-url"
        } else {
            "add"
        };
        git.run(
            "origin remote update",
            target,
            &["remote", action, "origin", url],
        )
        .await
        .map_err(|_| CheckoutFailureKind::Checkout)?;
    }
    checkout_requested_head(request, target, git).await
}

/// Builds the checkout for `request` directly from the remote, without a mirror.
async fn checkout_direct(
    request: &CheckoutRequest,
    url: &str,
    target: &Path,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    // Branch-only and pinned-commit requests start from an empty repository because a clone would
    // also download the default branch's history, which they do not want.
    if request.fetch_branch_only || matches!(request.head, Some(RepositoryHeadRef::CommitSha(_))) {
        git.run("checkout initialization", target, &["init", "--quiet"])
            .await
            .map_err(|_| CheckoutFailureKind::Clone)?;
        git.run(
            "origin remote creation",
            target,
            &["remote", "add", "origin", url],
        )
        .await
        .map_err(|_| CheckoutFailureKind::Clone)?;
    } else {
        clone_repository(request, url, target, git).await?;
    }
    checkout_requested_head(request, target, git).await
}

/// Creates a self-contained checkout of `request` that reuses the objects already in the local
/// mirror instead of downloading them again. The result does not depend on the mirror afterwards.
///
/// A branch-only request fetches just that branch from the remote, and every other request is
/// cloned from the mirror.
async fn checkout_cached(
    request: &CheckoutRequest,
    url: &str,
    target: &Path,
    mirror: &Path,
    default: &str,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    match (&request.head, request.fetch_branch_only) {
        (Some(RepositoryHeadRef::Branch(branch)), true) => {
            fetch_branch_from_mirror(branch, url, target, mirror, default, git).await
        }
        (Some(RepositoryHeadRef::CommitSha(_)) | None, true) => {
            git.record("branch-only checkout requires a branch");
            Err(CheckoutFailureKind::Checkout)
        }
        (head, false) => clone_from_mirror(head.as_ref(), url, target, mirror, git).await,
    }
}

/// Builds the checkout for a request that tracks every branch and tag by cloning the mirror and
/// then pointing `origin` at the remote. Unpinned requests end on a local branch of the default
/// branch, which is the mirror's `HEAD`; pinned requests are detached so `HEAD` is exactly the
/// requested head.
///
/// A clone records the remote-tracking refs it creates in `packed-refs` in one step. Fetching
/// the same refs writes a loose file for each, which fails for branches that differ only in case
/// on a case-insensitive filesystem. The clone hardlinks the mirror's objects, or copies them
/// when the mirror is on another filesystem, so the checkout does not depend on the mirror.
async fn clone_from_mirror(
    head: Option<&RepositoryHeadRef>,
    url: &str,
    target: &Path,
    mirror: &Path,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    let clone_error = CheckoutFailureKind::Clone;
    let checkout_error = CheckoutFailureKind::Checkout;
    let parent = target.parent().ok_or_else(|| {
        git.record("checkout target has no parent directory");
        clone_error
    })?;
    let source = mirror.to_str().ok_or_else(|| {
        git.record("cache mirror path is not valid UTF-8");
        clone_error
    })?;
    let destination = target.to_str().ok_or_else(|| {
        git.record("checkout target path is not valid UTF-8");
        clone_error
    })?;
    let mut args = vec!["clone", "--quiet"];
    if head.is_some() {
        args.push("--no-checkout");
    }
    args.extend(["--", source, destination]);
    git.run("cached clone", parent, &args)
        .await
        .map_err(|_| clone_error)?;
    git.run(
        "origin remote update",
        target,
        &["remote", "set-url", "origin", url],
    )
    .await
    .map_err(|_| clone_error)?;
    let result = match head {
        Some(RepositoryHeadRef::Branch(branch)) => {
            git.run(
                "requested head checkout",
                target,
                &[
                    "checkout",
                    "--detach",
                    &format!("refs/remotes/origin/{branch}"),
                ],
            )
            .await
        }
        Some(RepositoryHeadRef::CommitSha(sha)) => {
            // No branch or tag necessarily reaches a pinned commit, so the mirror may lack it.
            if git
                .run("requested head fetch", target, &["fetch", "origin", sha])
                .await
                .is_err()
            {
                return Err(checkout_error);
            }
            git.run(
                "requested head checkout",
                target,
                &["checkout", "--detach", sha],
            )
            .await
        }
        // A clone of a mirror whose default branch is missing succeeds without checking anything
        // out, which would otherwise go unnoticed.
        None => {
            git.run(
                "cached HEAD verification",
                target,
                &["rev-parse", "--verify", "HEAD"],
            )
            .await
        }
    };
    result.map(|_| ()).map_err(|_| checkout_error)
}

/// Builds the checkout for a branch-only request by fetching just that branch from the remote
/// and storing it under the default branch's tracking ref.
///
/// The mirror's objects are borrowed through `GIT_ALTERNATE_OBJECT_DIRECTORIES`, an environment
/// variable so that no `objects/info/alternates` file is left behind, and `repack` then copies
/// them into the checkout.
async fn fetch_branch_from_mirror(
    branch: &str,
    url: &str,
    target: &Path,
    mirror: &Path,
    default: &str,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    let clone_error = CheckoutFailureKind::Clone;
    let checkout_error = CheckoutFailureKind::Checkout;
    git.run(
        "cached checkout initialization",
        target,
        &["init", "--quiet"],
    )
    .await
    .map_err(|_| clone_error)?;
    git.run(
        "origin remote creation",
        target,
        &["remote", "add", "origin", url],
    )
    .await
    .map_err(|_| clone_error)?;

    let default_ref = format!("refs/remotes/origin/{default}");
    let refspec = format!("+refs/heads/{branch}:{default_ref}");
    git.run(
        "branch refspec configuration",
        target,
        &["config", "--replace-all", "remote.origin.fetch", &refspec],
    )
    .await
    .map_err(|_| checkout_error)?;
    let objects = mirror.join("objects");
    let alternates = [("GIT_ALTERNATE_OBJECT_DIRECTORIES", objects.as_os_str())];
    git.run_with_env(
        "cached fetch",
        target,
        &["fetch", "--no-tags", "origin"],
        &alternates,
    )
    .await
    .map_err(|_| checkout_error)?;
    git.run_with_env(
        "cached checkout",
        target,
        &["checkout", "--detach", &default_ref],
        &alternates,
    )
    .await
    .map_err(|_| checkout_error)?;
    // Fetching never sets `origin/HEAD`, which tools use to find the default branch.
    git.run(
        "origin HEAD update",
        target,
        &["symbolic-ref", "refs/remotes/origin/HEAD", &default_ref],
    )
    .await
    .map_err(|_| checkout_error)?;
    git.run_with_env(
        "cached object repack",
        target,
        &["repack", "-a", "-d"],
        &alternates,
    )
    .await
    .map_err(|_| clone_error)?;
    Ok(())
}

/// Clones the remote into `target`, deferring file contents until needed. When a head is
/// requested the working tree is left for the caller to move to it, and `fetch_branch_only`
/// limits the clone to that branch.
///
/// The clone is blobless (`--filter=blob:none`) so history is available without downloading every
/// file version up front. A requested head makes the clone skip its own checkout
/// (`--no-checkout`), because the default branch it would check out is not wanted.
async fn clone_repository(
    request: &CheckoutRequest,
    url: &str,
    target: &Path,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    let mut args = vec!["clone", "--filter=blob:none"];
    if request.head.is_some() {
        args.push("--no-checkout");
    }
    if request.fetch_branch_only {
        // Only the requested branch's history is wanted, without tags.
        args.extend([
            "--no-tags",
            "--single-branch",
            "--branch",
            request
                .head
                .as_ref()
                .ok_or_else(|| {
                    git.record("branch-only checkout requires a branch");
                    CheckoutFailureKind::Checkout
                })?
                .value(),
        ]);
    }
    args.extend([
        "--",
        url,
        target.to_str().ok_or_else(|| {
            git.record("checkout target path is not valid UTF-8");
            CheckoutFailureKind::Clone
        })?,
    ]);
    let parent = target.parent().ok_or_else(|| {
        git.record("checkout target has no parent directory");
        CheckoutFailureKind::Clone
    })?;
    git.run("clone", parent, &args)
        .await
        .map_err(|_| CheckoutFailureKind::Clone)?;
    Ok(())
}

/// Moves a repository that has an `origin` remote to the head requested by `request`, leaving
/// it detached. Does nothing when the request has no head.
///
/// A branch-only request fetches just the requested branch and stores it under the default
/// branch's tracking ref, so the repository holds a single remote-tracking branch that
/// `origin/HEAD` resolves to. Any other head is fetched by name, which also reaches commits that
/// no branch points to. Fetches are blobless so file contents are downloaded only for what gets
/// checked out.
async fn checkout_requested_head(
    request: &CheckoutRequest,
    target: &Path,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    let checkout_error = CheckoutFailureKind::Checkout;
    let Some(head) = &request.head else {
        return Ok(());
    };
    if request.fetch_branch_only {
        let default = remote_default_branch(target, git)
            .await
            .map_err(|_| checkout_error)?;
        let refspec = format!("+refs/heads/{}:refs/remotes/origin/{default}", head.value());
        git.run(
            "branch refspec configuration",
            target,
            &["config", "--replace-all", "remote.origin.fetch", &refspec],
        )
        .await
        .map_err(|_| checkout_error)?;
        // Tags are skipped because only the requested branch is wanted.
        git.run(
            "branch fetch",
            target,
            &["fetch", "--no-tags", "--filter=blob:none", "origin"],
        )
        .await
        .map_err(|_| checkout_error)?;
        // Detached, so the working tree is exactly the requested head and not a local branch
        // that could drift from it.
        git.run(
            "requested head checkout",
            target,
            &["checkout", "--detach", "FETCH_HEAD"],
        )
        .await
        .map_err(|_| checkout_error)?;
        // Fetching never sets `origin/HEAD`, which tools use to find the default branch.
        git.run(
            "origin HEAD update",
            target,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                &format!("refs/remotes/origin/{default}"),
            ],
        )
        .await
        .map_err(|_| checkout_error)?;
    } else {
        git.run(
            "requested head fetch",
            target,
            &["fetch", "--filter=blob:none", "origin", head.value()],
        )
        .await
        .map_err(|_| checkout_error)?;
        git.run(
            "requested head checkout",
            target,
            &["checkout", "--detach", "FETCH_HEAD"],
        )
        .await
        .map_err(|_| checkout_error)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "environment_checkout_tests.rs"]
mod tests;
