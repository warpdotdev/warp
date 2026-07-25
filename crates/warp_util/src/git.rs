use std::path::Path;

use anyhow::{anyhow, Result};

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "git_tests.rs"]
mod tests;

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

/// Builds the command that runs `git` with `args` in `repo_path`, with `env`
/// set on the child.
///
/// On a native Windows build, Warp represents the working directory of a WSL
/// session as a `\\wsl$\<distro>\...` UNC path. Running the Windows `git.exe`
/// against such a path is broken: it reports "dubious ownership", produces
/// bogus diffs, and can hang. For those paths the command is built as
/// `wsl.exe --distribution <distro> --cd <linux_path> --exec git <args...>`
/// instead, so the Linux-side git inside the distribution runs it. Every other
/// path gets a plain `git` command with `repo_path` as its working directory.
#[cfg(not(target_family = "wasm"))]
fn git_command(repo_path: &Path, args: &[&str], env: &[(&str, &str)]) -> command::r#async::Command {
    use command::r#async::Command;

    // The rewrite only applies to a Windows host. Gated with `cfg!` rather than
    // `#[cfg]` so the translation below stays compiled and unit-tested on every
    // platform.
    let translated = if cfg!(windows) {
        translate_for_wsl_unc_cwd(args, repo_path, env)
    } else {
        None
    };

    if let Some(translated) = translated {
        let mut cmd = Command::new("wsl.exe");
        cmd.args(&translated.args);
        // The working directory is deliberately left unset: `--cd` supplies it
        // inside the distribution, which keeps `wsl.exe` itself off the UNC
        // path. `PATH` is skipped here because it rides through the argument
        // vector instead (see [`translate_for_wsl_unc_cwd`]).
        for (key, value) in env.iter().filter(|(key, _)| !is_path_env_key(key)) {
            cmd.env(key, value);
        }
        if let Some(wslenv) = translated.wslenv {
            cmd.env("WSLENV", wslenv);
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

/// The `wsl.exe` invocation that replaces a `git` command whose working
/// directory lives inside a WSL distribution.
#[cfg(not(target_family = "wasm"))]
#[derive(Debug, PartialEq, Eq)]
struct WslGitCommand {
    /// The full argument vector for `wsl.exe`.
    args: Vec<String>,
    /// The value for the `WSLENV` variable that propagates the explicitly-set
    /// environment variables into the distribution, or `None` when there is
    /// nothing to propagate.
    wslenv: Option<String>,
}

/// Rewrites a `git` invocation whose working directory is a WSL UNC path into
/// the equivalent `wsl.exe` invocation. Returns `None` when `repo_path` is not
/// a WSL UNC path, in which case the caller runs `git` directly.
///
/// Non-`PATH` variables in `env` are advertised through `WSLENV` so they cross
/// into the distribution (see [`build_wslenv`]). An explicitly set `PATH` is
/// instead carried as an argv element:
/// `... --exec /usr/bin/env PATH=<value> git <args...>`. This is the `--exec`
/// analogue of the inline `PATH=...; cmd` assignment in
/// `app/src/terminal/model/session/command_executor/wsl_command_executor.rs`.
/// Routing `PATH` through argv bypasses Windows' non-disableable Windows-to-WSL
/// `PATH` conversion, which would otherwise truncate the caller-supplied
/// Linux-form `PATH` that `run_git_command_with_env` sets so hook tools such as
/// `git-lfs` resolve inside the distribution.
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
    // A caller-supplied `PATH` is prepended to the executed program as an `env`
    // assignment rather than propagated through `WSLENV`; see the rationale
    // above.
    if let Some((_, path_value)) = env.iter().find(|(key, _)| is_path_env_key(key)) {
        translated_args.push("/usr/bin/env".to_string());
        translated_args.push(format!("PATH={path_value}"));
    }
    translated_args.push("git".to_string());
    translated_args.extend(args.iter().map(|arg| translate_arg(arg, &unc.distro)));

    Some(WslGitCommand {
        args: translated_args,
        wslenv: build_wslenv(env),
    })
}

/// Rewrites a single argument: an argument that is itself a WSL UNC path for
/// the *same* distribution is converted to its Linux path, so paths passed to
/// git resolve inside the distribution. Arguments for other distributions and
/// non-UNC arguments are passed through unchanged.
#[cfg(not(target_family = "wasm"))]
fn translate_arg(arg: &str, distro: &str) -> String {
    match crate::path::parse_wsl_unc_path(Path::new(arg)) {
        Some(parsed) if parsed.distro.eq_ignore_ascii_case(distro) => parsed.linux_path,
        _ => arg.to_string(),
    }
}

/// Builds the `WSLENV` value that advertises the explicitly-set environment
/// variables to the distribution, using the `/u` suffix so each variable is
/// shared when invoking WSL from Windows. Returns `None` when no propagatable
/// variables were set.
///
/// `PATH` is deliberately excluded (case-insensitively): Windows applies a
/// non-disableable Windows-to-WSL `PATH` conversion, and a `PATH` that is
/// already in Linux form — as it is when a WSL session's environment is
/// threaded through [`run_git_command_with_env`] — fails that conversion and
/// gets truncated. `PATH` is instead carried as an argv element by
/// [`translate_for_wsl_unc_cwd`]. This mirrors the `PATH` handling in
/// `app/src/terminal/model/session/command_executor/wsl_command_executor.rs`.
#[cfg(not(target_family = "wasm"))]
fn build_wslenv(env: &[(&str, &str)]) -> Option<String> {
    let joined = env
        .iter()
        .map(|(key, _)| key)
        .filter(|key| !is_path_env_key(key))
        .map(|key| format!("{key}/u"))
        .collect::<Vec<_>>()
        .join(":");
    (!joined.is_empty()).then_some(joined)
}

/// True when `key` names the `PATH` environment variable, compared
/// case-insensitively. Used to keep a Linux-form `PATH` out of both `WSLENV`
/// and the environment handed to `wsl.exe` (see [`build_wslenv`]).
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
