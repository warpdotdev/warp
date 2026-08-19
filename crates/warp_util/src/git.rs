use std::path::Path;

use anyhow::{Result, anyhow};

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
    use command::Stdio;

    log::debug!(
        "[GIT OPERATION] git.rs run_git_command git {}",
        args.join(" ")
    );
    let mut git_args = vec!["-c", "diff.autoRefreshIndex=false"];
    git_args.extend_from_slice(args);
    let mut env = vec![("GIT_OPTIONAL_LOCKS", "0")];
    if let Some(path_env) = path_env {
        env.push(("PATH", path_env));
    }

    let mut cmd = git_command(repo_path, &git_args, &env);
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
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

/// Builds the command that runs `git` with `args` in `repo_path`, with `env` set on the child.
///
/// A WSL session's working directory is a `\\wsl$\<distro>\...` UNC path on a Windows host, and
/// the Windows `git.exe` mishandles those: it reports "dubious ownership", produces bogus diffs,
/// and can hang. Such a path is instead routed to the distribution's own git via `wsl.exe`.
#[cfg(not(target_family = "wasm"))]
fn git_command(repo_path: &Path, args: &[&str], env: &[(&str, &str)]) -> command::r#async::Command {
    use command::r#async::Command;

    // Gated with `cfg!` rather than `#[cfg]` so the translation stays compiled and unit-tested on
    // every platform.
    let translated = if cfg!(windows) {
        translate_for_wsl_unc_cwd(args, repo_path, env)
    } else {
        None
    };

    if let Some(translated) = translated {
        let mut cmd = Command::new("wsl.exe");
        cmd.args(&translated.args);
        // The working directory is deliberately left unset: `--cd` supplies it inside the
        // distribution, which keeps `wsl.exe` itself off the UNC path.
        // A caller-supplied `PATH` rides through the argument vector instead; see `build_wslenv`.
        for (key, value) in env.iter().filter(|(key, _)| !is_path_env_key(key)) {
            cmd.env(key, value);
        }
        // Left unset when empty so the child keeps inheriting the parent's `WSLENV`.
        if !translated.wslenv.is_empty() {
            cmd.env("WSLENV", &translated.wslenv);
        }
        return cmd;
    }

    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo_path);
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd
}

/// A `git` command rewritten to run inside a WSL distribution via `wsl.exe`.
#[cfg(not(target_family = "wasm"))]
#[derive(Debug, PartialEq, Eq)]
struct WslGitCommand {
    args: Vec<String>,
    /// The `WSLENV` value propagating the explicitly-set environment variables into the
    /// distribution; empty when there is nothing to propagate.
    wslenv: String,
}

/// Rewrites a `git` invocation whose working directory is a WSL UNC path into the equivalent
/// `wsl.exe` invocation, carrying `env` across as `WSLENV` entries except for `PATH`, which
/// becomes an argv element (`--exec /usr/bin/env PATH=<value> git ...`). Returns `None` when
/// `repo_path` is not a WSL UNC path.
#[cfg(not(target_family = "wasm"))]
fn translate_for_wsl_unc_cwd(
    args: &[&str],
    repo_path: &Path,
    env: &[(&str, &str)],
) -> Option<WslGitCommand> {
    let unc = crate::path::parse_wsl_unc_path(repo_path)?;

    let mut translated_args = vec![
        "--distribution".to_string(),
        unc.distro.clone(),
        "--cd".to_string(),
        unc.linux_path,
        "--exec".to_string(),
    ];
    match env.iter().find(|(key, _)| is_path_env_key(key)) {
        // A caller-supplied `PATH` already names the directory `git` lives in, so no login shell
        // is needed to resolve it.
        Some((_, path_value)) => {
            translated_args.push("/usr/bin/env".to_string());
            translated_args.push(format!("PATH={path_value}"));
            translated_args.push("git".to_string());
        }
        // Otherwise a login shell is needed: `wsl.exe --exec` searches only a minimal default
        // `PATH` (`/usr/bin`, `/bin`, ...), which misses distributions that put `git` elsewhere —
        // NixOS exposes it only under `/etc/profiles`. Arguments ride along as positional
        // parameters so no shell quoting is involved.
        None => {
            translated_args.push("/bin/sh".to_string());
            translated_args.push("-lc".to_string());
            translated_args.push(r#"exec git "$@""#.to_string());
            translated_args.push("git".to_string());
        }
    }
    translated_args.extend(args.iter().map(|arg| translate_arg(arg, &unc.distro)));

    Some(WslGitCommand {
        args: translated_args,
        wslenv: build_wslenv(env),
    })
}

/// Converts an argument that is a UNC path for `distro` into its Linux path. Every other argument
/// is passed through unchanged.
#[cfg(not(target_family = "wasm"))]
fn translate_arg(arg: &str, distro: &str) -> String {
    match crate::path::parse_wsl_unc_path(Path::new(arg)) {
        Some(parsed) if parsed.distro.eq_ignore_ascii_case(distro) => parsed.linux_path,
        _ => arg.to_string(),
    }
}

/// Builds the `WSLENV` value advertising the keys of `env` to the distribution, using the `/u`
/// suffix that shares a variable when invoking WSL from Windows. Empty when there is nothing to
/// propagate.
///
/// `PATH` is deliberately excluded: Windows applies a non-disableable Windows-to-WSL `PATH`
/// conversion, and a `PATH` that is already in Linux form fails that conversion and gets
/// truncated. It travels as an argv element instead.
#[cfg(not(target_family = "wasm"))]
fn build_wslenv(env: &[(&str, &str)]) -> String {
    env.iter()
        .map(|(key, _)| key)
        .filter(|key| !is_path_env_key(key))
        .map(|key| format!("{key}/u"))
        .collect::<Vec<_>>()
        .join(":")
}

/// True when `key` names the `PATH` environment variable, compared case-insensitively.
#[cfg(not(target_family = "wasm"))]
fn is_path_env_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("PATH")
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

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "git_tests.rs"]
mod tests;
