use std::collections::{HashMap, HashSet, VecDeque};
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
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt as _};
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef};
use warp_cli::environment_checkout::EnvironmentCheckoutArgs;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::cache_setup;
use super::environment_checkout_protocol::{
    CheckoutBatch, CheckoutFailure, CheckoutFailureKind, CheckoutFailureReport, CheckoutRequest,
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

pub(crate) fn run(args: &EnvironmentCheckoutArgs) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| sanitized_error("could not start checkout runtime", error))?
        .block_on(run_async(args))
}

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

async fn remove_origin(target: &Path, git: &mut Git) -> Result<(), ()> {
    if !fs::symlink_metadata(target)
        .await
        .map_err(|error| git.record_error("could not inspect origin cleanup target", error))?
        .is_dir()
    {
        git.record("origin cleanup target is not a repository directory");
        return Err(());
    }

    let remotes = git.run(target, &["remote"]).await?;
    if remotes.lines().any(|remote| remote == "origin") {
        // Removing the remote itself can collide on case-insensitive tracking-ref lock paths.
        git.run(
            target,
            &["config", "--local", "--remove-section", "remote.origin"],
        )
        .await?;
    }
    Ok(())
}

async fn run_async(args: &EnvironmentCheckoutArgs) -> anyhow::Result<()> {
    let bytes = fs::read(&args.requests_file)
        .await
        .map_err(|error| sanitized_error("could not read environment checkout requests", error))?;
    let batch = CheckoutBatch::parse(&bytes)
        .map_err(|error| sanitized_error("could not parse environment checkout requests", error))?;
    let mirror_root = (!args.remove_origins_only)
        .then(optional_mirror_root)
        .flatten();
    let results = checkout_batch(
        &batch,
        mirror_root.as_deref(),
        args.remove_origins_only,
        log::logger(),
    )
    .await?;
    let failures = results
        .into_iter()
        .enumerate()
        .filter_map(|(request_index, result)| {
            result.err().map(|(kind, output)| CheckoutFailure {
                request_index,
                kind,
                output,
            })
        })
        .collect::<Vec<_>>();
    let failed = !failures.is_empty();
    let identity_diagnostics = if failed && !args.remove_origins_only {
        Some(collect_failure_identity(&batch, &failures).await)
    } else {
        None
    };
    let bytes = serde_json::to_vec(&CheckoutFailureReport {
        failures,
        identity_diagnostics,
    })?;
    fs::write(&args.failure_report, bytes)
        .await
        .map_err(|error| {
            sanitized_error("could not write environment checkout failure report", error)
        })?;
    if failed {
        Err(anyhow!("structured repository checkout failed"))
    } else {
        if !args.remove_origins_only
            && let Some(path) = &args.resolved_heads_report
        {
            let heads = capture_resolved_heads(&batch).await;
            if let Err(error) = fs::write(path, serde_json::to_vec(&heads)?).await {
                log::warn!(
                    "{}",
                    sanitized_error(
                        "could not write structured repository resolved HEAD report",
                        error
                    )
                );
            }
        }
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

async fn capture_resolved_heads(batch: &CheckoutBatch) -> Vec<Option<String>> {
    let capture = async {
        let mut heads = Vec::new();
        for request in &batch.repositories {
            let mut git = Git::default();
            let head = git
                .run(
                    &batch.working_dir.join(&request.checkout_name),
                    &["rev-parse", "--verify", "HEAD"],
                )
                .await
                .ok()
                .and_then(|head| parse_resolved_head_sha(&head));
            if let Some(head) = &head {
                log::info!(
                    "Repository {}: resolved HEAD {head}",
                    repository_label(request)
                );
            } else {
                log::warn!(
                    "Repository {}: could not resolve HEAD\n{}",
                    repository_label(request),
                    git.diagnostics
                );
            }
            heads.push(head);
        }
        heads
    };
    tokio::time::timeout(HEAD_CAPTURE_TIMEOUT, capture)
        .await
        .unwrap_or_else(|_| {
            log::warn!("Timed out capturing structured repository resolved HEADs");
            vec![None; batch.repositories.len()]
        })
}

async fn identity_query(cwd: &Path, args: &[&str], input: Option<&str>) -> Option<String> {
    tokio::time::timeout(IDENTITY_QUERY_TIMEOUT, async {
        let mut child = Command::new("git")
            .current_dir(cwd)
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

async fn collect_failure_identity(
    batch: &CheckoutBatch,
    failures: &[CheckoutFailure],
) -> CloneFailureIdentityDiagnostics {
    let mut seen = HashSet::new();
    let hosts = failures
        .iter()
        .map(|failure| {
            source_repo(&batch.repositories[failure.request_index])
                .code_forge
                .unwrap_or_default()
                .host()
        })
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

fn optional_mirror_root() -> Option<PathBuf> {
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

fn mirror_key(request: &CheckoutRequest) -> RepoCacheKey {
    let repo = source_repo(request);
    RepoCacheKey::derive(&RepoIdentity::new(
        repo.code_forge.unwrap_or_default().host(),
        repo.owner,
        repo.repo,
    ))
}

type CheckoutResult = Result<(), (CheckoutFailureKind, String)>;

async fn checkout_batch(
    batch: &CheckoutBatch,
    mirror_root: Option<&Path>,
    remove_origins_only: bool,
    logger: &dyn log::Log,
) -> anyhow::Result<Vec<CheckoutResult>> {
    batch.validate().map_err(|error| anyhow!(error))?;
    let mut groups = HashMap::<_, Vec<_>>::new();
    for (index, request) in batch.repositories.iter().enumerate() {
        groups
            .entry(mirror_key(request))
            .or_default()
            .push((index, request));
    }
    let mut results = vec![None; batch.repositories.len()];
    let mut completed = stream::iter(groups.into_values().map(|requests| async move {
        let mut results = Vec::new();
        for (index, request) in requests {
            let label = repository_label(request);
            let emit = |category: &str| {
                log::info!(logger: logger, "Repository {label}: {category}");
            };
            emit("started");
            let mut git = Git::default();
            let result = if remove_origins_only {
                remove_origin(&batch.working_dir.join(&request.checkout_name), &mut git)
                    .await
                    .map_err(|_| CheckoutFailureKind::RemoveOrigin)
            } else {
                checkout(request, &batch.working_dir, mirror_root, &mut git, &emit).await
            };
            emit(if result.is_ok() { "finished" } else { "failed" });
            results.push((index, result.map_err(|kind| (kind, git.diagnostics))));
        }
        results
    }))
    .buffer_unordered(CHECKOUT_WORKERS);
    while let Some(group) = completed.next().await {
        for (index, result) in group {
            results[index] = Some(result);
        }
    }
    let results = results
        .into_iter()
        .map(|result| result.expect("checkout worker result"))
        .collect::<Vec<_>>();
    for (index, result) in results.iter().enumerate() {
        if let Err((_, diagnostics)) = result {
            log::error!(logger: logger,
                "Repository {} diagnostics:\n{diagnostics}",
                repository_label(&batch.repositories[index])
            );
        }
    }
    Ok(results)
}

#[derive(Default)]
struct Git {
    diagnostics: String,
}

impl Git {
    async fn run(&mut self, cwd: &Path, args: &[&str]) -> Result<String, ()> {
        let operation = args.first().copied().unwrap_or("command");
        let child = Command::new("git")
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
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

    fn record(&mut self, output: &str) {
        let combined = format!("{}\n{output}", self.diagnostics);
        self.diagnostics =
            failure_output::prepare_failure_output(&combined, OUTPUT_TRUNCATION_MARKER);
    }

    fn record_error(&mut self, context: &str, error: impl fmt::Display) {
        self.record(&format!("{context}: {error:#}"));
    }
}

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

async fn checkout(
    request: &CheckoutRequest,
    working_dir: &Path,
    mirror_root: Option<&Path>,
    git: &mut Git,
    emit: &impl Fn(&str),
) -> Result<(), CheckoutFailureKind> {
    let target = working_dir.join(&request.checkout_name);
    let existing = match fs::symlink_metadata(&target).await {
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
        let mut created_target = false;
        let cached = async {
            refresh_mirror(&mirror, &url, git, emit).await?;
            fs::create_dir(&target).await.map_err(|error| {
                git.record_error("could not create cached checkout target", error)
            })?;
            created_target = true;
            checkout_cached(request, &url, &target, &mirror, git)
                .await
                .map_err(|_| ())
        }
        .await;
        if cached.is_ok() {
            return Ok(());
        }
        emit("cache failure; using network fallback");
        if !git.diagnostics.is_empty() {
            emit(&format!("cache failure diagnostics:\n{}", git.diagnostics));
        }
        if created_target {
            fs::remove_dir_all(&target).await.map_err(|error| {
                git.record_error("could not remove failed cached checkout target", error);
                CheckoutFailureKind::Clone
            })?;
        }
        git.diagnostics.clear();
    }
    fs::create_dir(&target).await.map_err(|error| {
        git.record_error("could not create checkout target", error);
        CheckoutFailureKind::Clone
    })?;
    checkout_network(request, &url, &target, git).await
}

async fn refresh_mirror(
    mirror: &Path,
    url: &str,
    git: &mut Git,
    emit: &impl Fn(&str),
) -> Result<(), ()> {
    let parent = mirror
        .parent()
        .ok_or_else(|| git.record("cache mirror has no parent directory"))?;
    let existing = fs::symlink_metadata(mirror).await;
    let path = mirror
        .to_str()
        .ok_or_else(|| git.record("cache mirror path is not valid UTF-8"))?;
    let config_path = mirror.join("config");
    let config_path_str = config_path
        .to_str()
        .ok_or_else(|| git.record("cache mirror config path is not valid UTF-8"))?;
    let canonical_config = format!(
        "[core]\n\trepositoryformatversion = 0\n\tbare = true\n\
         [remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/heads/*\n"
    );
    let mut valid = existing.as_ref().is_ok_and(|metadata| metadata.is_dir())
        && git
            .run(
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
            .run(mirror, &["rev-parse", "--is-bare-repository"])
            .await
            .is_ok_and(|value| value.trim() == "true");
    }
    if valid {
        emit("cache hit");
    } else {
        match existing {
            Ok(metadata) => {
                emit("invalid cache; rebuilding");
                if !metadata.is_dir() {
                    fs::remove_file(mirror).await.map_err(|error| {
                        git.record_error("could not remove invalid cache mirror file", error)
                    })?;
                } else {
                    fs::remove_dir_all(mirror).await.map_err(|error| {
                        git.record_error("could not remove invalid cache mirror directory", error)
                    })?;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => emit("cold cache population"),
            Err(error) => {
                git.record_error("could not inspect cache mirror", error);
                return Err(());
            }
        }
        git.run(parent, &["init", "--bare", "--quiet", path])
            .await?;
        fs::write(&config_path, &canonical_config)
            .await
            .map_err(|error| git.record_error("could not write new cache mirror config", error))?;
    }
    git.run(
        mirror,
        &[
            "fetch",
            "--prune",
            "--prune-tags",
            "origin",
            "+refs/heads/*:refs/heads/*",
            "+refs/tags/*:refs/tags/*",
        ],
    )
    .await?;
    let head = remote_default_branch(mirror, git).await?;
    git.run(
        mirror,
        &["symbolic-ref", "HEAD", &format!("refs/heads/{head}")],
    )
    .await?;
    Ok(())
}

async fn remote_default_branch(target: &Path, git: &mut Git) -> Result<String, ()> {
    let output = git
        .run(target, &["ls-remote", "--symref", "origin", "HEAD"])
        .await?;
    let branch = output
        .lines()
        .find_map(|line| {
            line.strip_prefix("ref: refs/heads/")
                .and_then(|value| value.strip_suffix("\tHEAD"))
        })
        .ok_or_else(|| git.record("Git ls-remote did not report a symbolic default branch"))?;
    git.run(target, &["check-ref-format", "--branch", branch])
        .await?;
    Ok(branch.to_owned())
}

async fn checkout_existing(
    request: &CheckoutRequest,
    url: &str,
    target: &Path,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    if request.fetch_branch_only {
        let action = if git
            .run(target, &["remote", "get-url", "origin"])
            .await
            .is_ok()
        {
            "set-url"
        } else {
            "add"
        };
        git.run(target, &["remote", action, "origin", url])
            .await
            .map_err(|_| CheckoutFailureKind::Checkout)?;
    }
    checkout_requested_head(request, target, true, git)
        .await
        .map(|_| ())
}

async fn checkout_network(
    request: &CheckoutRequest,
    url: &str,
    target: &Path,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    if request.fetch_branch_only || matches!(request.head, Some(RepositoryHeadRef::CommitSha(_))) {
        git.run(target, &["init", "--quiet"])
            .await
            .map_err(|_| CheckoutFailureKind::Clone)?;
        git.run(target, &["remote", "add", "origin", url])
            .await
            .map_err(|_| CheckoutFailureKind::Clone)?;
    } else {
        clone_repository(request, url, target, &["--filter=blob:none"], git).await?;
    }
    checkout_requested_head(request, target, true, git)
        .await
        .map(|_| ())
}

async fn checkout_cached(
    request: &CheckoutRequest,
    url: &str,
    target: &Path,
    mirror: &Path,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    let reference = mirror.to_str().ok_or_else(|| {
        git.record("cache mirror reference path is not valid UTF-8");
        CheckoutFailureKind::Clone
    })?;
    clone_repository(
        request,
        url,
        target,
        &["--reference", reference, "--dissociate"],
        git,
    )
    .await?;
    if let Some(default) = checkout_requested_head(request, target, false, git).await?
        && let Some(head) = &request.head
        && head.value() != default
    {
        git.run(
            target,
            &[
                "update-ref",
                "-d",
                &format!("refs/remotes/origin/{}", head.value()),
            ],
        )
        .await
        .map_err(|_| CheckoutFailureKind::Checkout)?;
    }
    Ok(())
}

async fn clone_repository(
    request: &CheckoutRequest,
    url: &str,
    target: &Path,
    options: &[&str],
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    let mut args = vec!["clone"];
    if request.head.is_some() {
        args.push("--no-checkout");
    }
    args.extend(options);
    if request.fetch_branch_only {
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
    git.run(parent, &args)
        .await
        .map_err(|_| CheckoutFailureKind::Clone)?;
    Ok(())
}

async fn checkout_requested_head(
    request: &CheckoutRequest,
    target: &Path,
    blobless: bool,
    git: &mut Git,
) -> Result<Option<String>, CheckoutFailureKind> {
    let checkout_error = CheckoutFailureKind::Checkout;
    let Some(head) = &request.head else {
        return Ok(None);
    };
    if request.fetch_branch_only {
        let default = remote_default_branch(target, git)
            .await
            .map_err(|_| checkout_error)?;
        let refspec = format!("+refs/heads/{}:refs/remotes/origin/{default}", head.value());
        git.run(
            target,
            &["config", "--replace-all", "remote.origin.fetch", &refspec],
        )
        .await
        .map_err(|_| checkout_error)?;
        let mut fetch = vec!["fetch", "--no-tags"];
        if blobless {
            fetch.push("--filter=blob:none");
        }
        fetch.push("origin");
        git.run(target, &fetch).await.map_err(|_| checkout_error)?;
        git.run(target, &["checkout", "--detach", "FETCH_HEAD"])
            .await
            .map_err(|_| checkout_error)?;
        git.run(
            target,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                &format!("refs/remotes/origin/{default}"),
            ],
        )
        .await
        .map_err(|_| checkout_error)?;
        Ok(Some(default))
    } else {
        let mut fetch = vec!["fetch"];
        if blobless {
            fetch.push("--filter=blob:none");
        }
        fetch.extend(["origin", head.value()]);
        git.run(target, &fetch).await.map_err(|_| checkout_error)?;
        git.run(target, &["checkout", "--detach", "FETCH_HEAD"])
            .await
            .map_err(|_| checkout_error)?;
        Ok(None)
    }
}

#[cfg(test)]
#[path = "environment_checkout_tests.rs"]
mod tests;
