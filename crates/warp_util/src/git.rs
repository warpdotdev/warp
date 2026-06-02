use std::path::Path;

use anyhow::{anyhow, Result};

/// Runs a git command and returns the output as a string.
/// Thin wrapper over [`run_git_command_with_env`] with no `PATH` override.
#[cfg(not(target_family = "wasm"))]
pub async fn run_git_command(repo_path: &Path, args: &[&str]) -> Result<String> {
    run_git_command_with_env(repo_path, args, None).await
}

/// Like [`run_git_command`] but sets `PATH` on the child when `path_env` is
/// `Some`. Used by callers whose hooks need user-installed binaries (e.g.
/// the LFS `pre-push` hook → `git-lfs`). See `specs/APP-4188/TECH.md`.
#[cfg(not(target_family = "wasm"))]
pub async fn run_git_command_with_env(
    repo_path: &Path,
    args: &[&str],
    path_env: Option<&str>,
) -> Result<String> {
    use command::r#async::Command;
    use command::Stdio;

    log::debug!(
        "[GIT OPERATION] git.rs run_git_command git {}",
        args.join(" ")
    );
    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg("diff.autoRefreshIndex=false")
        .args(args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .kill_on_drop(true);
    if let Some(path_env) = path_env {
        cmd.env("PATH", path_env);
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| anyhow!("Failed to execute git command: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Handle git diff specific behavior:
    // - Exit code 0: no differences
    // - Exit code 1: differences found (this is normal for diff commands)
    // - Exit code > 1: actual error
    if output.status.success() || (output.status.code() == Some(1) && !stdout.is_empty()) {
        Ok(stdout)
    } else {
        Err(anyhow!("Git command failed: {}, {}", stderr, stdout))
    }
}

#[cfg(target_family = "wasm")]
pub async fn run_git_command(_repo_path: &Path, _args: &[&str]) -> Result<String> {
    Err(anyhow!("Not supported on wasm"))
}

#[cfg(target_family = "wasm")]
pub async fn run_git_command_with_env(
    _repo_path: &Path,
    _args: &[&str],
    _path_env: Option<&str>,
) -> Result<String> {
    Err(anyhow!("Not supported on wasm"))
}
