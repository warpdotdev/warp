use std::path::Path;

use anyhow::{Result, anyhow};

/// Runs a git command and returns the output as a string.
/// Thin wrapper over [`run_git_command_with_env`] with no `PATH` override.
#[cfg(not(target_family = "wasm"))]
pub async fn run_git_command(repo_path: &Path, args: &[&str]) -> Result<String> {
    run_git_command_with_env(repo_path, args, None).await
}

/// Chunk size used when incrementally reading a subprocess's stdout/stderr in
/// [`run_git_command_capped`].
#[cfg(not(target_family = "wasm"))]
const CAPPED_READ_CHUNK_SIZE: usize = 64 * 1024;

/// Diagnostic cap on a capped git command's stderr capture (see
/// [`run_git_command_capped`]). Git's own error output is normally a few
/// hundred bytes; this budget is generous for unusually verbose failures
/// while still bounding a misbehaving diff driver (e.g. a `textconv` helper,
/// which `--no-ext-diff` does not disable) that writes to stderr without
/// limit — the same unbounded-growth hazard this file fixes for stdout.
#[cfg(not(target_family = "wasm"))]
const STDERR_CAPTURE_CAP: usize = 256 * 1024;

/// Outcome of [`run_git_command_capped`].
#[derive(Debug)]
pub enum CappedGitOutput {
    /// The complete stdout, decoded lossily as UTF-8; the byte budget was not
    /// exceeded.
    Complete(String),
    /// The subprocess's stdout exceeded the byte budget before it finished
    /// writing. The child was killed rather than left to keep writing an
    /// arbitrarily large output, so no output is returned.
    Exceeded,
}

/// Outcome of reading a single pipe up to a byte budget via [`read_capped`]
/// or [`read_two_capped`].
#[cfg(not(target_family = "wasm"))]
enum CappedReadOutcome {
    /// The pipe reached EOF within the budget; carries everything read.
    Complete(Vec<u8>),
    /// Accumulated bytes exceeded the budget before EOF, so reading stopped
    /// immediately instead of continuing to EOF. Carries what was read up to
    /// the overshoot (bounded by `max_bytes + CAPPED_READ_CHUNK_SIZE`), kept
    /// only for diagnostics — callers must not treat this as complete output.
    Exceeded(Vec<u8>),
}

/// Reads `reader` in fixed-size chunks, stopping as soon as the accumulated
/// length exceeds `max_bytes` rather than continuing on to EOF. This is the
/// core guarantee behind [`run_git_command_capped`]'s memory bound: the cap
/// is enforced incrementally during the read, not by checking a
/// fully-buffered length afterward (see APP-5462).
#[cfg(not(target_family = "wasm"))]
async fn read_capped<R>(reader: &mut R, max_bytes: usize) -> std::io::Result<CappedReadOutcome>
where
    R: futures_lite::io::AsyncRead + Unpin,
{
    use futures_lite::io::AsyncReadExt;

    let mut buf = Vec::with_capacity(CAPPED_READ_CHUNK_SIZE);
    // Heap-allocated rather than a `[u8; CAPPED_READ_CHUNK_SIZE]` stack array:
    // a fixed-size array crossing an `.await` point is embedded inline in
    // this function's generated future, and that future is itself embedded
    // in every caller's future all the way up (`get_file_diff` ->
    // `diff_state_against_head` -> the spawned diff-load task). A 64KB
    // array there adds 64KB to every one of those stack frames whenever the
    // combined future is polled on a real (non-boxed) call stack, which is
    // exactly the kind of bloat that can overflow a worker thread's stack
    // deep inside the real app, well before it would show up in a shallow
    // unit test.
    let mut chunk = vec![0u8; CAPPED_READ_CHUNK_SIZE];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Ok(CappedReadOutcome::Complete(buf));
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > max_bytes {
            return Ok(CappedReadOutcome::Exceeded(buf));
        }
    }
}

/// Reads two pipes concurrently, each bounded by its own byte budget, and
/// stops as soon as *either* pipe exceeds its budget — without waiting for
/// the other to reach EOF. That matters for both directions: an oversized
/// diff overflows stdout, and a misbehaving diff driver (e.g. `textconv`)
/// can hold stderr open indefinitely even after the main child has exited,
/// so waiting for stderr's own EOF is not safe once stdout (or stderr) has
/// already blown its budget.
///
/// Returns each pipe's outcome, or `None` for a pipe that was still
/// mid-read when the other pipe's overflow ended the operation.
#[cfg(not(target_family = "wasm"))]
async fn read_two_capped<R1, R2>(
    mut first: R1,
    first_budget: usize,
    mut second: R2,
    second_budget: usize,
) -> std::io::Result<(Option<CappedReadOutcome>, Option<CappedReadOutcome>)>
where
    R1: futures_lite::io::AsyncRead + Unpin,
    R2: futures_lite::io::AsyncRead + Unpin,
{
    use std::future::Future;
    use std::pin::pin;
    use std::task::Poll;

    // Built directly on `read_capped` — each pipe's future *is* the same
    // incremental-stop logic the single-pipe path uses and tests, driven
    // manually here only to interleave the two pipes.
    let mut first_fut = pin!(read_capped(&mut first, first_budget));
    let mut second_fut = pin!(read_capped(&mut second, second_budget));
    let mut first_result: Option<std::io::Result<CappedReadOutcome>> = None;
    let mut second_result: Option<std::io::Result<CappedReadOutcome>> = None;

    // Poll both futures on every tick, not just the first until it blocks,
    // so a pipe that's always immediately ready (e.g. a descendant that
    // never blocks) can't stall progress on the other. Stop as soon as
    // either pipe errors or exceeds its budget, without waiting for the
    // other to reach EOF: a misbehaving diff driver (e.g. `textconv`) can
    // hold a pipe open indefinitely even after the main child has exited.
    futures_lite::future::poll_fn(|cx| {
        if first_result.is_none()
            && let Poll::Ready(result) = first_fut.as_mut().poll(cx)
        {
            first_result = Some(result);
        }
        if matches!(
            first_result,
            Some(Err(_)) | Some(Ok(CappedReadOutcome::Exceeded(_)))
        ) {
            return Poll::Ready(());
        }

        if second_result.is_none()
            && let Poll::Ready(result) = second_fut.as_mut().poll(cx)
        {
            second_result = Some(result);
        }
        if matches!(
            second_result,
            Some(Err(_)) | Some(Ok(CappedReadOutcome::Exceeded(_)))
        ) {
            return Poll::Ready(());
        }

        if first_result.is_some() && second_result.is_some() {
            return Poll::Ready(());
        }
        Poll::Pending
    })
    .await;

    let first_outcome = first_result.transpose()?;
    let second_outcome = second_result.transpose()?;
    Ok((first_outcome, second_outcome))
}

/// Like [`run_git_command`], but bounds both the subprocess's stdout *and*
/// stderr capture instead of buffering either in full before deciding
/// whether it's usable. Reads both incrementally in fixed-size chunks and
/// kills the child as soon as either budget is exceeded, rather than
/// waiting for it to finish writing arbitrarily large output on either pipe
/// (see APP-5462).
///
/// Only meant for commands whose output can legitimately be enormous (e.g.
/// `git diff` on a single huge file). Silently truncating output would be a
/// correctness hazard for most other git subcommands (e.g. `git show`, ref
/// listings), so this is not the default entry point — see
/// [`run_git_command`].
#[cfg(not(target_family = "wasm"))]
pub async fn run_git_command_capped(
    repo_path: &Path,
    args: &[&str],
    max_bytes: usize,
) -> Result<CappedGitOutput> {
    use command::Stdio;

    log::debug!(
        "[GIT OPERATION] git.rs run_git_command_capped git {}",
        args.join(" ")
    );
    let mut git_args = vec!["-c", "diff.autoRefreshIndex=false"];
    git_args.extend_from_slice(args);
    let env = [("GIT_OPTIONAL_LOCKS", "0")];

    let mut cmd = git_command(repo_path, &git_args, &env);
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("Failed to execute git command: {}", e))?;
    let stdout = child
        .stdout
        .take()
        .expect("stdout is configured as piped above");
    let stderr = child
        .stderr
        .take()
        .expect("stderr is configured as piped above");

    let (stdout_outcome, stderr_outcome) =
        match read_two_capped(stdout, max_bytes, stderr, STDERR_CAPTURE_CAP).await {
            Ok(outcomes) => outcomes,
            Err(e) => {
                let _ = child.kill();
                let _ = child.status().await;
                return Err(anyhow!("Failed to read git command output: {}", e));
            }
        };

    if let Some(CappedReadOutcome::Exceeded(partial_stderr)) = stderr_outcome {
        // A misbehaving diff driver (e.g. textconv) can write unbounded
        // stderr; kill and reap rather than waiting for the pipe to close on
        // its own, which a still-running descendant could delay
        // indefinitely. The full error text is only preserved while it fits
        // the diagnostic cap; here it doesn't, so this is truncated by
        // construction (`partial_stderr` is bounded by
        // `STDERR_CAPTURE_CAP + CAPPED_READ_CHUNK_SIZE`).
        let _ = child.kill();
        let _ = child.status().await;
        return Err(anyhow!(
            "Git command failed: stderr exceeded {} bytes and was truncated: {}",
            STDERR_CAPTURE_CAP,
            String::from_utf8_lossy(&partial_stderr)
        ));
    }
    if matches!(stdout_outcome, Some(CappedReadOutcome::Exceeded(_))) {
        // Stop reading and kill the child instead of waiting for it to
        // finish writing an arbitrarily large diff.
        let _ = child.kill();
        let _ = child.status().await;
        return Ok(CappedGitOutput::Exceeded);
    }

    let (
        Some(CappedReadOutcome::Complete(stdout_bytes)),
        Some(CappedReadOutcome::Complete(stderr_bytes)),
    ) = (stdout_outcome, stderr_outcome)
    else {
        // Unreachable in practice: read_two_capped only leaves a pipe's
        // outcome as anything other than `Complete` when it (or the other
        // pipe) exceeded its budget, both of which are handled above.
        let _ = child.kill();
        let _ = child.status().await;
        return Err(anyhow!(
            "Failed to read git command output: incomplete capture"
        ));
    };

    let status = child
        .status()
        .await
        .map_err(|e| anyhow!("Failed to wait for git command: {}", e))?;
    let stdout_str = String::from_utf8_lossy(&stdout_bytes).to_string();
    let stderr_str = String::from_utf8_lossy(&stderr_bytes);

    // Mirrors run_git_command_with_env's git-diff-specific exit code handling.
    if status.success() || (status.code() == Some(1) && !stdout_str.is_empty()) {
        Ok(CappedGitOutput::Complete(stdout_str))
    } else {
        Err(anyhow!(
            "Git command failed: {}, {}",
            stderr_str,
            stdout_str
        ))
    }
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

#[cfg(target_family = "wasm")]
pub async fn run_git_command_capped(
    _repo_path: &Path,
    _args: &[&str],
    _max_bytes: usize,
) -> Result<CappedGitOutput> {
    Err(anyhow!("Not supported on wasm"))
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "git_tests.rs"]
mod tests;
