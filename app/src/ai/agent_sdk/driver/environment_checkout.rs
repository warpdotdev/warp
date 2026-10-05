use std::collections::HashMap;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fs, thread};

use anyhow::anyhow;
use build_cache::{RepoCacheKey, RepoIdentity};
use cloud_object_models::{CodeForge, SourceRepo};
use command::Stdio;
use command::blocking::Command;
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

pub(crate) fn run(args: &EnvironmentCheckoutArgs) -> anyhow::Result<()> {
    let bytes = fs::read(&args.requests_file)
        .map_err(|_| anyhow!("could not read environment checkout requests"))?;
    let batch = CheckoutBatch::parse(&bytes).map_err(|error| anyhow!(error))?;
    let mirror_root = optional_mirror_root();
    let results = checkout_batch(&batch, mirror_root.as_deref(), &mut io::stdout())?;
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
        .map_err(|_| anyhow!("could not write environment checkout failure report"))?;
    if failed {
        Err(anyhow!("structured repository checkout failed"))
    } else {
        Ok(())
    }
}

fn optional_mirror_root() -> Option<PathBuf> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let cache_root = cache_setup::enabled_cache_root()?;
        let root = cache_root.join("git-mirrors");
        let metadata = fs::symlink_metadata(&root);
        if metadata
            .as_ref()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
            || metadata.as_ref().is_ok_and(|metadata| !metadata.is_dir())
        {
            return None;
        }
        fs::create_dir_all(&root).ok()?;
        tempfile::NamedTempFile::new_in(&root).ok()?;
        fs::canonicalize(root).ok()
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

fn checkout_batch(
    batch: &CheckoutBatch,
    mirror_root: Option<&Path>,
    output: &mut (impl io::Write + Send),
) -> anyhow::Result<Vec<CheckoutResult>> {
    batch.validate().map_err(|error| anyhow!(error))?;
    let locks = batch
        .repositories
        .iter()
        .map(|request| (mirror_key(request), Mutex::new(())))
        .collect::<HashMap<_, _>>();
    let next = AtomicUsize::new(0);
    let progress = Mutex::new(output);
    let results = Mutex::new(vec![None; batch.repositories.len()]);
    thread::scope(|scope| {
        for _ in 0..CHECKOUT_WORKERS.min(batch.repositories.len()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(request) = batch.repositories.get(index) else {
                        break;
                    };
                    let emit = |category: &str| {
                        let mut output = progress.lock().expect("checkout progress lock");
                        let _ = writeln!(output, "Repository {}: {category}", index + 1);
                        let _ = output.flush();
                    };
                    emit("started");
                    let key = mirror_key(request);
                    let _lock =
                        mirror_root.map(|_| locks[&key].lock().expect("checkout mirror lock"));
                    let mut git = Git::default();
                    let result =
                        checkout(request, &batch.working_dir, mirror_root, &mut git, &emit);
                    emit(if result.is_ok() { "finished" } else { "failed" });
                    results.lock().expect("checkout results lock")[index] =
                        Some(result.map_err(|kind| (kind, git.diagnostics)));
                }
            });
        }
    });
    let results = results
        .into_inner()
        .expect("checkout results lock")
        .into_iter()
        .map(|result| result.expect("checkout worker result"))
        .collect::<Vec<_>>();
    for (index, result) in results.iter().enumerate() {
        if let Err((_, diagnostics)) = result {
            let mut output = progress.lock().expect("checkout progress lock");
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
    fn run(&mut self, cwd: &Path, args: &[&str]) -> Result<String, ()> {
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
        let (stdout, stderr, status) = thread::scope(|scope| {
            let stdout = scope.spawn(|| capture(stdout));
            let stderr = scope.spawn(|| capture(stderr));
            (
                stdout.join().expect("Git stdout reader"),
                stderr.join().expect("Git stderr reader"),
                child.wait(),
            )
        });
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

fn capture(mut reader: impl io::Read) -> io::Result<(String, bool)> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(CAPTURE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > CAPTURE_BYTES;
    io::copy(&mut reader, &mut io::sink())?;
    if truncated {
        bytes.truncate(bytes.iter().rposition(|byte| *byte == b'\n').unwrap_or(0));
    }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
}

fn checkout(
    request: &CheckoutRequest,
    working_dir: &Path,
    mirror_root: Option<&Path>,
    git: &mut Git,
    emit: &impl Fn(&str),
) -> Result<(), CheckoutFailureKind> {
    let target = working_dir.join(&request.checkout_name);
    let existing = match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => true,
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
        return materialize(request, &url, &target, true, None, git);
    }
    if let Some(root) = mirror_root {
        let mirror = root.join(mirror_key(request).as_str());
        let mut created_target = false;
        let cached = refresh_mirror(&mirror, &url, git, emit).and_then(|()| {
            fs::create_dir(&target).map_err(|_| ())?;
            created_target = true;
            materialize(request, &url, &target, false, Some(&mirror), git).map_err(|_| ())
        });
        if cached.is_ok() {
            return Ok(());
        }
        emit("cache failure; using network fallback");
        if created_target {
            fs::remove_dir_all(&target).map_err(|_| CheckoutFailureKind::Clone)?;
        }
        git.diagnostics.clear();
    }
    fs::create_dir(&target).map_err(|_| {
        git.record("could not create checkout target");
        CheckoutFailureKind::Clone
    })?;
    materialize(request, &url, &target, false, None, git)
}

fn refresh_mirror(mirror: &Path, url: &str, git: &mut Git, emit: &impl Fn(&str)) -> Result<(), ()> {
    let parent = mirror.parent().ok_or(())?;
    if fs::symlink_metadata(parent)
        .map_err(|_| ())?
        .file_type()
        .is_symlink()
    {
        return Err(());
    }
    let existing = fs::symlink_metadata(mirror);
    let valid = existing.is_ok()
        && walkdir::WalkDir::new(mirror)
            .follow_links(false)
            .into_iter()
            .all(|entry| entry.is_ok_and(|entry| !entry.file_type().is_symlink()))
        && git
            .run(mirror, &["rev-parse", "--is-bare-repository"])
            .is_ok_and(|value| value.trim() == "true")
        && git
            .run(
                mirror,
                &[
                    "config",
                    "--local",
                    "--no-includes",
                    "--get",
                    "remote.origin.url",
                ],
            )
            .is_ok_and(|value| value.trim() == url)
        && git
            .run(mirror, &["config", "--local", "--no-includes", "--list"])
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
                        && !name.starts_with("include")
                        && !name.starts_with("url.")
                        && name != "core.worktree"
                        && !name.starts_with("credential.")
                        && !name.starts_with("http.")
                        && !name.starts_with("https.")
                        && (!name.starts_with("remote.")
                            || matches!(name.as_str(), "remote.origin.url" | "remote.origin.fetch"))
                })
            });
    if valid {
        emit("cache hit");
    } else {
        match existing {
            Ok(metadata) => {
                emit("invalid cache; rebuilding");
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    fs::remove_file(mirror).map_err(|_| ())?;
                } else {
                    fs::remove_dir_all(mirror).map_err(|_| ())?;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => emit("cold cache population"),
            Err(_) => return Err(()),
        }
        let path = mirror.to_str().ok_or(())?;
        git.run(parent, &["init", "--bare", "--quiet", path])?;
        git.run(mirror, &["remote", "add", "origin", url])?;
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
    )?;
    let head = remote_default_branch(mirror, git)?;
    git.run(
        mirror,
        &["symbolic-ref", "HEAD", &format!("refs/heads/{head}")],
    )?;
    Ok(())
}

fn remote_default_branch(target: &Path, git: &mut Git) -> Result<String, ()> {
    let output = git.run(target, &["ls-remote", "--symref", "origin", "HEAD"])?;
    let branch = output
        .lines()
        .find_map(|line| {
            line.strip_prefix("ref: refs/heads/")
                .and_then(|value| value.strip_suffix("\tHEAD"))
        })
        .ok_or(())?;
    git.run(target, &["check-ref-format", "--branch", branch])?;
    Ok(branch.to_owned())
}

fn materialize(
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
                .map_err(|_| clone_error)?;
            git.run(target, &["remote", "add", "origin", url])
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
                .map_err(|_| clone_error)?;
        }
    }
    let Some(head) = &request.head else {
        return Ok(());
    };
    if request.fetch_branch_only {
        if existing {
            let action = if git.run(target, &["remote", "get-url", "origin"]).is_ok() {
                "set-url"
            } else {
                "add"
            };
            git.run(target, &["remote", action, "origin", url])
                .map_err(|_| checkout_error)?;
        }
        let default = remote_default_branch(target, git).map_err(|_| checkout_error)?;
        let refspec = format!("+refs/heads/{}:refs/remotes/origin/{default}", head.value());
        git.run(
            target,
            &["config", "--replace-all", "remote.origin.fetch", &refspec],
        )
        .map_err(|_| checkout_error)?;
        let mut fetch = vec!["fetch", "--no-tags"];
        if mirror.is_none() {
            fetch.push("--filter=blob:none");
        }
        fetch.push("origin");
        git.run(target, &fetch).map_err(|_| checkout_error)?;
        git.run(target, &["checkout", "--detach", "FETCH_HEAD"])
            .map_err(|_| checkout_error)?;
        git.run(
            target,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                &format!("refs/remotes/origin/{default}"),
            ],
        )
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
            .map_err(|_| checkout_error)?;
        }
    } else {
        let mut fetch = vec!["fetch"];
        if mirror.is_none() {
            fetch.push("--filter=blob:none");
        }
        fetch.extend(["origin", head.value()]);
        git.run(target, &fetch).map_err(|_| checkout_error)?;
        git.run(target, &["checkout", "--detach", "FETCH_HEAD"])
            .map_err(|_| checkout_error)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "environment_checkout_tests.rs"]
mod tests;
