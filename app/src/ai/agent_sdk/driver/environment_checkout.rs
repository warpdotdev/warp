use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use async_compat::Compat;
use build_cache::metadata::CacheUsage;
use build_cache::{RepoCacheKey, RepoIdentity};
use cloud_object_models::{CodeForge, SourceRepo};
use command::Stdio;
use command::r#async::Command;
use futures::{StreamExt as _, stream};
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt as _};
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef};
use warp_cli::environment_checkout::{
    CheckoutBatch, CheckoutFailure, CheckoutFailureKind, CheckoutFailureReport, CheckoutRequest,
    EnvironmentCheckoutArgs,
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::cache_setup;
use super::failure_output;

const CHECKOUT_WORKERS: usize = 4;
const CAPTURE_BYTES: usize = 64 * 1024;
const OUTPUT_TRUNCATION_MARKER: &str = "\n… git output truncated …\n";
const MIRRORS_DIRECTORY: &str = "git-mirrors";

pub(crate) fn run(args: &EnvironmentCheckoutArgs) -> anyhow::Result<()> {
    futures::executor::block_on(Compat::new(run_async(args)))
}

async fn remove_origin(target: &Path, git: &mut Git) -> Result<(), ()> {
    if !fs::symlink_metadata(target).await.map_err(|_| ())?.is_dir() {
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
        .map_err(|_| anyhow!("could not read environment checkout requests"))?;
    let batch = CheckoutBatch::parse(&bytes).map_err(|error| anyhow!(error))?;
    let mirror_root = (!args.remove_origins_only)
        .then(optional_mirror_root)
        .flatten();
    let results = checkout_batch(
        &batch,
        mirror_root.as_deref(),
        args.remove_origins_only,
        &mut io::stdout(),
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
    let bytes = serde_json::to_vec(&CheckoutFailureReport { failures })?;
    fs::write(&args.failure_report, bytes)
        .await
        .map_err(|_| anyhow!("could not write environment checkout failure report"))?;
    if failed {
        Err(anyhow!("structured repository checkout failed"))
    } else {
        Ok(())
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
            return None;
        }
        std::fs::create_dir_all(&root).ok()?;
        let _writability_probe = tempfile::NamedTempFile::new_in(&root).ok()?;
        std::fs::canonicalize(root).ok()
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
    output: &mut impl io::Write,
) -> anyhow::Result<Vec<CheckoutResult>> {
    batch.validate().map_err(|error| anyhow!(error))?;
    let mut groups = HashMap::<_, Vec<_>>::new();
    for (index, request) in batch.repositories.iter().enumerate() {
        groups
            .entry(mirror_key(request))
            .or_default()
            .push((index, request));
    }
    let progress = RefCell::new(output);
    let mut results = vec![None; batch.repositories.len()];
    let mut completed = stream::iter(groups.into_values().map(|requests| {
        let progress = &progress;
        async move {
            let mut results = Vec::new();
            for (index, request) in requests {
                let emit = |category: &str| {
                    let mut output = progress.borrow_mut();
                    let _ = writeln!(output, "Repository {}: {category}", index + 1);
                    let _ = output.flush();
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
        }
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
            let mut output = progress.borrow_mut();
            writeln!(
                output,
                "Repository {} diagnostics:\n{diagnostics}",
                index + 1
            )?;
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
        let child = Command::new("git")
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let Ok(mut child) = child else {
            self.record("could not start Git");
            return Err(());
        };
        let stdout = child.stdout.take().expect("Git stdout pipe");
        let stderr = child.stderr.take().expect("Git stderr pipe");
        let (stdout, stderr, status) = tokio::join!(
            capture(Compat::new(stdout)),
            capture(Compat::new(stderr)),
            child.status(),
        );
        let (Ok((stdout, truncated)), Ok((stderr, _)), Ok(status)) = (stdout, stderr, status)
        else {
            self.record("could not read Git result");
            return Err(());
        };
        self.record(&stderr);
        if status.success() && !truncated {
            Ok(stdout)
        } else {
            self.record(&stdout);
            Err(())
        }
    }

    fn record(&mut self, output: &str) {
        let combined = format!("{}\n{output}", self.diagnostics);
        self.diagnostics =
            failure_output::prepare_failure_output(&combined, OUTPUT_TRUNCATION_MARKER);
    }
}

async fn capture(mut reader: impl AsyncRead + Unpin) -> io::Result<(String, bool)> {
    let mut bytes = Vec::new();
    (&mut reader)
        .take(CAPTURE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    let truncated = bytes.len() > CAPTURE_BYTES;
    tokio::io::copy(&mut reader, &mut tokio::io::sink()).await?;
    if truncated {
        bytes.truncate(bytes.iter().rposition(|byte| *byte == b'\n').unwrap_or(0));
    }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
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
        Err(_) => {
            git.record("could not inspect checkout target");
            return Err(CheckoutFailureKind::Clone);
        }
    };
    let url = source_repo(request).https_clone_url();
    if existing {
        return materialize(request, &url, &target, true, None, git).await;
    }
    if let Some(root) = mirror_root {
        let mirror = root.join(mirror_key(request).as_str());
        let mut created_target = false;
        let cached = async {
            refresh_mirror(&mirror, &url, git, emit).await?;
            fs::create_dir(&target).await.map_err(|_| ())?;
            created_target = true;
            materialize(request, &url, &target, false, Some(&mirror), git)
                .await
                .map_err(|_| ())
        }
        .await;
        if cached.is_ok() {
            return Ok(());
        }
        emit("cache failure; using network fallback");
        if created_target {
            fs::remove_dir_all(&target)
                .await
                .map_err(|_| CheckoutFailureKind::Clone)?;
        }
        git.diagnostics.clear();
    }
    fs::create_dir(&target).await.map_err(|_| {
        git.record("could not create checkout target");
        CheckoutFailureKind::Clone
    })?;
    materialize(request, &url, &target, false, None, git).await
}

async fn refresh_mirror(
    mirror: &Path,
    url: &str,
    git: &mut Git,
    emit: &impl Fn(&str),
) -> Result<(), ()> {
    let parent = mirror.parent().ok_or(())?;
    let existing = fs::symlink_metadata(mirror).await;
    let path = mirror.to_str().ok_or(())?;
    let config_path = mirror.join("config");
    let canonical_config = format!(
        "[core]\n\trepositoryformatversion = 0\n\tbare = true\n\
         [remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/heads/*\n"
    );
    let inspected = mirror.to_owned();
    let full_tree = tokio::task::spawn_blocking(move || {
        walkdir::WalkDir::new(inspected)
            .follow_links(false)
            .into_iter()
            .all(|entry| {
                entry.is_ok_and(|entry| {
                    !entry.file_type().is_symlink()
                        && entry
                            .path()
                            .extension()
                            .is_none_or(|extension| extension != "promisor")
                })
            })
    })
    .await
    .map_err(|_| ())?;
    let mut valid = existing.as_ref().is_ok_and(|metadata| metadata.is_dir())
        && full_tree
        && git
            .run(
                parent,
                &[
                    "config",
                    "--file",
                    config_path.to_str().ok_or(())?,
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
            .map_err(|_| ())?;
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
                    fs::remove_file(mirror).await.map_err(|_| ())?;
                } else {
                    fs::remove_dir_all(mirror).await.map_err(|_| ())?;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => emit("cold cache population"),
            Err(_) => return Err(()),
        }
        git.run(parent, &["init", "--bare", "--quiet", path])
            .await?;
        fs::write(&config_path, &canonical_config)
            .await
            .map_err(|_| ())?;
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
        .ok_or(())?;
    git.run(target, &["check-ref-format", "--branch", branch])
        .await?;
    Ok(branch.to_owned())
}

async fn materialize(
    request: &CheckoutRequest,
    url: &str,
    target: &Path,
    existing: bool,
    mirror: Option<&Path>,
    git: &mut Git,
) -> Result<(), CheckoutFailureKind> {
    let clone_error = CheckoutFailureKind::Clone;
    let checkout_error = CheckoutFailureKind::Checkout;
    if !existing {
        if mirror.is_none()
            && (request.fetch_branch_only
                || matches!(request.head, Some(RepositoryHeadRef::CommitSha(_))))
        {
            git.run(target, &["init", "--quiet"])
                .await
                .map_err(|_| clone_error)?;
            git.run(target, &["remote", "add", "origin", url])
                .await
                .map_err(|_| clone_error)?;
        } else {
            let target_path = target.to_str().ok_or(clone_error)?;
            let mut args = vec!["clone"];
            if request.head.is_some() {
                args.push("--no-checkout");
            }
            let reference;
            if let Some(mirror) = mirror {
                reference = mirror.to_str().ok_or(clone_error)?;
                args.extend(["--reference", reference, "--dissociate"]);
            } else {
                args.push("--filter=blob:none");
            }
            if request.fetch_branch_only {
                args.extend([
                    "--no-tags",
                    "--single-branch",
                    "--branch",
                    request.head.as_ref().ok_or(checkout_error)?.value(),
                ]);
            }
            args.extend(["--", url, target_path]);
            git.run(target.parent().ok_or(clone_error)?, &args)
                .await
                .map_err(|_| clone_error)?;
        }
    }
    let Some(head) = &request.head else {
        return Ok(());
    };
    if request.fetch_branch_only {
        if existing {
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
                .map_err(|_| checkout_error)?;
        }
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
        if mirror.is_none() {
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
        if !existing && mirror.is_some() && head.value() != default {
            git.run(
                target,
                &[
                    "update-ref",
                    "-d",
                    &format!("refs/remotes/origin/{}", head.value()),
                ],
            )
            .await
            .map_err(|_| checkout_error)?;
        }
    } else {
        let mut fetch = vec!["fetch"];
        if mirror.is_none() {
            fetch.push("--filter=blob:none");
        }
        fetch.extend(["origin", head.value()]);
        git.run(target, &fetch).await.map_err(|_| checkout_error)?;
        git.run(target, &["checkout", "--detach", "FETCH_HEAD"])
            .await
            .map_err(|_| checkout_error)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "environment_checkout_tests.rs"]
mod tests;
