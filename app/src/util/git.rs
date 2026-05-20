use std::collections::HashSet;
use std::path::Path;

use anyhow::{anyhow, Result};
use warp_core::safe_warn;
use warp_util::git::run_git_command;
#[cfg(feature = "local_fs")]
use warp_util::git::run_git_command_with_env;

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;

/// Returns the set of local branch names for the repo at `repo_path`.
/// Uses a synchronous subprocess call — suitable for call sites in
/// synchronous view handlers where the result is needed immediately.
/// Returns an empty set on any failure (not a git repo, git not found, etc.).
#[cfg(feature = "local_fs")]
pub fn list_local_branches_sync(repo_path: &Path) -> HashSet<String> {
    let output = command::blocking::Command::new("git")
        .args(["branch", "--list", "--format=%(refname:short)"])
        .current_dir(repo_path)
        .stdout(command::Stdio::piped())
        .stderr(command::Stdio::null())
        .output();

    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect(),
        _ => HashSet::new(),
    }
}

#[cfg(not(feature = "local_fs"))]
pub fn list_local_branches_sync(_repo_path: &Path) -> HashSet<String> {
    HashSet::new()
}

/// Fetches the current git branch.
#[cfg(not(feature = "local_fs"))]
pub async fn detect_current_branch(_repo_path: &Path) -> Result<String> {
    Err(anyhow!("Not supported without local_fs"))
}

/// Fetches the current git branch.
/// In detached HEAD state this returns the literal string "HEAD".
#[cfg(feature = "local_fs")]
pub async fn detect_current_branch(repo_path: &Path) -> Result<String> {
    log::debug!("[GIT OPERATION] git.rs detect_current_branch git rev-parse --abbrev-ref HEAD");
    let result = run_git_command(repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]).await;

    if result.is_err() {
        log::debug!("[GIT OPERATION] git.rs detect_current_branch git branch --show-current");
        run_git_command(repo_path, &["branch", "--show-current"]).await
    } else {
        result
    }
    .map(|branch_name| branch_name.trim().to_owned())
}

/// Like [`detect_current_branch`], but in detached HEAD state returns the short
/// commit SHA instead of the literal "HEAD".
/// (Matches the shell command `git symbolic-ref --short HEAD || git rev-parse --short HEAD`.)
#[cfg(feature = "local_fs")]
pub async fn detect_current_branch_display(repo_path: &Path) -> Result<String> {
    let branch = detect_current_branch(repo_path).await?;
    if branch == "HEAD" {
        run_git_command(repo_path, &["rev-parse", "--short", "HEAD"])
            .await
            .map(|sha| sha.trim().to_owned())
    } else {
        Ok(branch)
    }
}

#[cfg(not(feature = "local_fs"))]
pub async fn detect_current_branch_display(_repo_path: &Path) -> Result<String> {
    Err(anyhow!("Not supported without local_fs"))
}

/// Detects the main branch using git-branchless style heuristics.
#[cfg(not(feature = "local_fs"))]
pub async fn detect_main_branch(_repo_path: &Path) -> Result<String> {
    Err(anyhow!("Not supported without local_fs"))
}

/// Detects the main branch using git-branchless style heuristics.
#[cfg(feature = "local_fs")]
pub async fn detect_main_branch(repo_path: &Path) -> Result<String> {
    // First try to get the default branch from origin
    log::debug!(
        "[GIT OPERATION] git.rs detect_main_branch git symbolic-ref refs/remotes/origin/HEAD"
    );
    match run_git_command(repo_path, &["symbolic-ref", "refs/remotes/origin/HEAD"]).await {
        Ok(output) => {
            let branch_ref = output.trim();
            if let Some(branch_name) = branch_ref.strip_prefix("refs/remotes/") {
                return Ok(branch_name.to_string());
            }
        }
        Err(_) => {
            // If remote fetch fails, fall back to candidates.
        }
    }

    // Fallback: try common main branch names in order of preference.
    let candidates = ["origin/main", "origin/master", "main", "master", "develop"];

    for candidate in candidates {
        log::debug!(
            "[GIT OPERATION] git.rs detect_main_branch git rev-parse --verify {candidate}^{{}}"
        );
        let result = run_git_command(
            repo_path,
            &["rev-parse", "--verify", &format!("{candidate}^{{}}")],
        )
        .await;

        if result.is_ok() {
            return Ok(candidate.to_string());
        }
    }

    // Final fallback if all else fails.
    log::debug!("[GIT OPERATION] git.rs detect_main_branch git branch --show-current");
    run_git_command(repo_path, &["branch", "--show-current"]).await
}

/// Returns the SHA where `HEAD` forked from any other ref. Use
/// `<fork>..HEAD` for "commits unique to this branch".
#[cfg(not(feature = "local_fs"))]
pub async fn detect_fork_point(
    _repo_path: &Path,
    _current_branch_name: Option<&str>,
) -> Result<Option<String>> {
    Err(anyhow!("Not supported without local_fs"))
}

/// See the no-`local_fs` stub above for documentation.
#[cfg(feature = "local_fs")]
pub async fn detect_fork_point(
    repo_path: &Path,
    current_branch_name: Option<&str>,
) -> Result<Option<String>> {
    // Exclude `<current>` and `origin/<current>` so the branch isn't
    // subtracted from itself.
    let current = current_branch_name
        .map(str::trim)
        .filter(|branch| !branch.is_empty() && *branch != "HEAD");

    let branch_exclude = current.map(|c| format!("--exclude={c}"));
    let remote_exclude = current.map(|c| format!("--exclude=origin/{c}"));

    let mut args: Vec<&str> = vec!["rev-list", "HEAD", "--not"];
    args.extend(branch_exclude.as_deref());
    args.push("--branches");
    args.extend(remote_exclude.as_deref());
    args.push("--remotes");

    let unique = match run_git_command(repo_path, &args).await {
        Ok(out) => out,
        Err(e) => {
            log::debug!("detect_fork_point: rev-list failed: {e}");
            return Ok(None);
        }
    };

    // Last non-empty line = oldest unique commit; its parent = fork point.
    // No unique commits means HEAD is fully shared, so fork = HEAD.
    let target = match unique.lines().rfind(|l| !l.trim().is_empty()) {
        Some(sha) => format!("{}^", sha.trim()),
        None => "HEAD".to_string(),
    };
    Ok(run_git_command(repo_path, &["rev-parse", &target])
        .await
        .ok()
        .map(|s| s.trim().to_string()))
}

/// Git summary for a repo: current branch + uncommitted diff stats.
#[derive(Debug, Clone)]
#[cfg(feature = "local_fs")]
pub struct RepoGitSummary {
    pub branch: String,
    pub lines_added: u32,
    pub lines_removed: u32,
}

/// Runs git commands in `repo_root` to get current branch + diff stats.
/// Returns None if not a git repo or git is unavailable.
#[cfg(feature = "local_fs")]
pub async fn get_repo_git_summary(repo_root: &Path) -> Option<RepoGitSummary> {
    use crate::context_chips::display_chip::GitLineChanges;

    let branch = {
        log::debug!("[GIT OPERATION] git.rs get_repo_git_summary git symbolic-ref --short HEAD");
        let result = run_git_command(repo_root, &["symbolic-ref", "--short", "HEAD"]).await;
        match result {
            Ok(output) => Some(output.trim().to_string()),
            Err(_) => {
                // Fallback to rev-parse for detached HEAD
                log::debug!(
                    "[GIT OPERATION] git.rs get_repo_git_summary git rev-parse --short HEAD"
                );
                run_git_command(repo_root, &["rev-parse", "--short", "HEAD"])
                    .await
                    .ok()
                    .map(|o| o.trim().to_string())
            }
        }
    };

    // Tracked file changes (git diff --shortstat HEAD doesn't include untracked files).
    log::debug!("[GIT OPERATION] git.rs get_repo_git_summary git diff --shortstat HEAD");
    let stats = run_git_command(repo_root, &["diff", "--shortstat", "HEAD"])
        .await
        .ok()
        .and_then(|o| GitLineChanges::parse_from_git_output(&o));

    let mut lines_added = stats.as_ref().map_or(0, |s| s.lines_added);
    let lines_removed = stats.as_ref().map_or(0, |s| s.lines_removed);

    // Also count lines in untracked files to match what the git diff chip shows.
    log::debug!(
        "[GIT OPERATION] git.rs get_repo_git_summary git ls-files --others --exclude-standard"
    );
    if let Ok(untracked_output) =
        run_git_command(repo_root, &["ls-files", "--others", "--exclude-standard"]).await
    {
        for file_name in untracked_output.lines() {
            if file_name.is_empty() {
                continue;
            }
            lines_added += count_lines_if_text_file(&repo_root.join(file_name));
        }
    }

    let branch = branch?;
    Some(RepoGitSummary {
        branch,
        lines_added,
        lines_removed,
    })
}

/// Short summary of a commit: hash and subject line.
#[derive(Debug, Clone)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub files_changed: usize,
    pub additions: usize,
    pub deletions: usize,
}

/// A single changed file with per-file addition/deletion counts.
#[derive(Debug, Clone)]
pub struct FileChangeEntry {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
}

/// Returns per-file change entries. When `include_unstaged` is true, returns all
/// uncommitted changes (staged + unstaged + untracked) vs HEAD; otherwise only staged changes.
#[cfg(feature = "local_fs")]
pub async fn get_file_change_entries(
    repo_path: &Path,
    include_unstaged: bool,
) -> Result<Vec<FileChangeEntry>> {
    let args: &[&str] = if include_unstaged {
        &["diff", "--numstat", "HEAD"]
    } else {
        &["diff", "--cached", "--numstat"]
    };
    let output = run_git_command(repo_path, args).await.unwrap_or_default();
    let mut entries = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            entries.push(FileChangeEntry {
                path: parts[2].to_string(),
                additions: parts[0].parse().unwrap_or(0),
                deletions: parts[1].parse().unwrap_or(0),
            });
        }
    }

    // Also include untracked files when showing all changes.
    if include_unstaged {
        if let Ok(untracked) =
            run_git_command(repo_path, &["ls-files", "--others", "--exclude-standard"]).await
        {
            for file_name in untracked.lines() {
                if file_name.is_empty() {
                    continue;
                }
                let additions = count_lines_if_text_file(&repo_path.join(file_name)) as usize;
                entries.push(FileChangeEntry {
                    path: file_name.to_string(),
                    additions,
                    deletions: 0,
                });
            }
        }
    }

    Ok(entries)
}

#[cfg(not(feature = "local_fs"))]
pub async fn get_file_change_entries(
    _repo_path: &Path,
    _include_unstaged: bool,
) -> Result<Vec<FileChangeEntry>> {
    Err(anyhow!("Not supported on wasm"))
}

/// Unpushed commits: `<upstream>..HEAD`, or `<fork_point>..HEAD` if no upstream.
#[cfg(feature = "local_fs")]
pub async fn get_unpushed_commits(
    repo_path: &Path,
    current_branch_name: Option<&str>,
    upstream_ref: Option<&str>,
) -> Result<Vec<Commit>> {
    let output = if let Some(upstream_ref) = upstream_ref.map(str::trim).filter(|s| !s.is_empty()) {
        let range = format!("{upstream_ref}..HEAD");
        run_git_command(
            repo_path,
            &["log", &range, "--format=COMMIT:%H\t%s", "--numstat"],
        )
        .await?
    } else {
        // No upstream — fall back to the fork-point commit so we show
        // exactly the commits unique to this branch
        let fork_point = detect_fork_point(repo_path, current_branch_name)
            .await
            .ok()
            .flatten();

        let range = match fork_point {
            Some(sha) => format!("{sha}..HEAD"),
            None => "HEAD".to_string(),
        };

        run_git_command(
            repo_path,
            &["log", &range, "--format=COMMIT:%H\t%s", "--numstat"],
        )
        .await
        .inspect_err(|e| log::warn!("Fallback unpushed-commits log failed: {e}"))
        .unwrap_or_default()
    };
    parse_commit_log(&output)
}

#[cfg(feature = "local_fs")]
fn parse_commit_log(output: &str) -> Result<Vec<Commit>> {
    let mut commits = Vec::new();
    let mut current: Option<Commit> = None;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("COMMIT:") {
            if let Some(commit) = current.take() {
                commits.push(commit);
            }
            let parts: Vec<&str> = rest.splitn(2, '\t').collect();
            if parts.len() == 2 {
                current = Some(Commit {
                    hash: parts[0].to_string(),
                    subject: parts[1].to_string(),
                    files_changed: 0,
                    additions: 0,
                    deletions: 0,
                });
            }
        } else if !line.is_empty() {
            // numstat line: "<additions>\t<deletions>\t<path>"
            if let Some(ref mut commit) = current {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if parts.len() == 3 {
                    commit.additions += parts[0].parse::<usize>().unwrap_or(0);
                    commit.deletions += parts[1].parse::<usize>().unwrap_or(0);
                    commit.files_changed += 1;
                }
            }
        }
    }

    if let Some(commit) = current {
        commits.push(commit);
    }

    Ok(commits)
}

#[cfg(not(feature = "local_fs"))]
pub async fn get_unpushed_commits(
    _repo_path: &Path,
    _current_branch_name: Option<&str>,
    _upstream_ref: Option<&str>,
) -> Result<Vec<Commit>> {
    Err(anyhow!("Not supported on wasm"))
}

/// Returns the list of files changed in a specific commit, with per-file stats.
#[cfg(feature = "local_fs")]
pub async fn get_commit_files(repo_path: &Path, hash: &str) -> Result<Vec<FileChangeEntry>> {
    let output = run_git_command(
        repo_path,
        &[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "-r",
            "--numstat",
            hash,
        ],
    )
    .await?;

    let mut entries = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() == 3 {
            entries.push(FileChangeEntry {
                path: parts[2].to_string(),
                additions: parts[0].parse().unwrap_or(0),
                deletions: parts[1].parse().unwrap_or(0),
            });
        }
    }

    Ok(entries)
}

#[cfg(not(feature = "local_fs"))]
pub async fn get_commit_files(_repo_path: &Path, _hash: &str) -> Result<Vec<FileChangeEntry>> {
    Err(anyhow!("Not supported on wasm"))
}

/// Maximum number of characters of diff content to send to AI for commit
/// message / PR title / PR description generation.
#[cfg(feature = "local_fs")]
const MAX_DIFF_CHARS_FOR_AI: usize = 16_000;

/// Per-file cap for untracked-file content we synthesise into the diff sent
/// to AI. Keeps any one new file from dominating the budget.
#[cfg(feature = "local_fs")]
const MAX_UNTRACKED_FILE_BYTES: usize = 4_000;

/// Number of leading bytes examined when classifying an untracked file as
/// binary, mirroring the heuristic in `count_lines_if_text_file`.
#[cfg(feature = "local_fs")]
const BINARY_CHECK_BYTES: usize = 1_024;

/// Maximum number of bytes in a PR title passed to `gh pr create`. GitHub's
/// hard limit is 256; we cap short of that to leave headroom for an
/// ellipsis marker. Measured in bytes because it's fed to
/// [`truncate_on_char_boundary`], which slices on byte offsets.
#[cfg(feature = "local_fs")]
const MAX_PR_TITLE_BYTES: usize = 200;

/// Returns a prefix of `s` whose length is at most `byte_cap` and which ends
/// on a UTF-8 char boundary. Plain `&s[..byte_cap]` panics when the cut
/// point lands inside a multi-byte code point, which is reachable in diffs
/// and source files containing non-ASCII text.
#[cfg(feature = "local_fs")]
fn truncate_on_char_boundary(s: &str, byte_cap: usize) -> &str {
    if s.len() <= byte_cap {
        return s;
    }
    let mut cut = byte_cap;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

/// Returns the diff for commit message generation, truncated to avoid token
/// limits. When `include_unstaged` is true, diffs against HEAD (all
/// uncommitted changes) and also appends untracked files as synthetic diff
/// hunks so the LLM has full context even when the commit consists entirely
/// of new files. When `include_unstaged` is false, diffs only staged changes.
#[cfg(feature = "local_fs")]
pub async fn get_diff_for_commit_message(
    repo_path: &Path,
    include_unstaged: bool,
) -> Result<String> {
    let mut diff = if !include_unstaged {
        run_git_command(repo_path, &["diff", "--cached"]).await?
    } else if run_git_command(repo_path, &["rev-parse", "--verify", "HEAD"])
        .await
        .is_ok()
    {
        run_git_command(repo_path, &["diff", "HEAD"]).await?
    } else {
        // No HEAD before the first commit. Include staged changes plus
        // unstaged edits to staged files; untracked files are added below.
        let mut diff = run_git_command(repo_path, &["diff", "--cached"]).await?;
        diff.push_str(&run_git_command(repo_path, &["diff"]).await?);
        diff
    };

    // `git diff HEAD` only shows changes to already-tracked files. New files that
    // haven't been staged yet are invisible to it, so we synthesise diff hunks for
    // them here — mirroring the logic in `get_file_change_entries`.
    if include_unstaged {
        if let Ok(untracked) = run_git_command(
            repo_path,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )
        .await
        {
            // `-z` separates paths with NUL bytes and disables C-style
            // quoting, so paths containing spaces or non-ASCII characters
            // round-trip intact.
            // Cap the read to cover both the binary-check window and the
            // synthesised-hunk budget.
            let read_cap = BINARY_CHECK_BYTES.max(MAX_UNTRACKED_FILE_BYTES);
            for file_name_bytes in untracked.as_bytes().split(|b| *b == 0) {
                if file_name_bytes.is_empty() {
                    continue;
                }
                let Ok(file_name) = std::str::from_utf8(file_name_bytes) else {
                    continue;
                };
                let file_path = repo_path.join(file_name);
                // Async + bounded so a large untracked file doesn't block
                // the executor or balloon memory.
                let Ok(file) = tokio::fs::File::open(&file_path).await else {
                    continue;
                };
                let mut bytes = Vec::with_capacity(read_cap);
                use tokio::io::AsyncReadExt as _;
                if file
                    .take(read_cap as u64)
                    .read_to_end(&mut bytes)
                    .await
                    .is_err()
                {
                    continue;
                }
                let check_len = bytes.len().min(BINARY_CHECK_BYTES);
                if warp_util::file_type::is_buffer_binary(&bytes[..check_len]) {
                    continue;
                }
                let Ok(content) = std::str::from_utf8(&bytes) else {
                    continue;
                };
                let content = truncate_on_char_boundary(content, MAX_UNTRACKED_FILE_BYTES);
                let line_count = content.lines().count();
                diff.push_str(&format!(
                    "diff --git a/{file_name} b/{file_name}\nnew file mode 100644\n\
                     --- /dev/null\n+++ b/{file_name}\n@@ -0,0 +1,{line_count} @@\n"
                ));
                for line in content.lines() {
                    diff.push('+');
                    diff.push_str(line);
                    diff.push('\n');
                }
            }
        }
    }

    if diff.len() <= MAX_DIFF_CHARS_FOR_AI {
        Ok(diff)
    } else {
        Ok(format!(
            "{}\n... (diff truncated)",
            truncate_on_char_boundary(&diff, MAX_DIFF_CHARS_FOR_AI)
        ))
    }
}

#[cfg(not(feature = "local_fs"))]
pub async fn get_diff_for_commit_message(
    _repo_path: &Path,
    _include_unstaged: bool,
) -> Result<String> {
    Err(anyhow!("Not supported on wasm"))
}

/// Commits changes. If `include_unstaged` is true, stages all changes first via `git add -A`.
/// `path_env` is forwarded so commit hooks can find tools on the user's `PATH`.
#[cfg(feature = "local_fs")]
pub async fn run_commit(
    repo_path: &Path,
    message: &str,
    include_unstaged: bool,
    path_env: Option<&str>,
) -> Result<String> {
    if include_unstaged {
        run_git_command_with_env(repo_path, &["add", "-A"], path_env).await?;
    }
    run_git_command_with_env(repo_path, &["commit", "-m", message], path_env).await
}

#[cfg(not(feature = "local_fs"))]
pub async fn run_commit(
    _repo_path: &Path,
    _message: &str,
    _include_unstaged: bool,
    _path_env: Option<&str>,
) -> Result<String> {
    Err(anyhow!("Not supported on wasm"))
}

/// Per-file stats for what would land in a PR: default branch vs
/// `origin/<current>` (or HEAD when unpushed).
#[cfg(feature = "local_fs")]
pub async fn get_branch_diff_entries(repo_path: &Path) -> Result<Vec<FileChangeEntry>> {
    let base = detect_main_branch(repo_path).await?;
    let base = base.trim();
    let current = detect_current_branch(repo_path).await?;
    let remote_ref = format!("origin/{current}");

    // Use the remote ref if it exists, otherwise fall back to HEAD.
    let end_ref = if run_git_command(repo_path, &["rev-parse", "--verify", &remote_ref])
        .await
        .is_ok()
    {
        remote_ref
    } else {
        "HEAD".to_string()
    };

    let range = format!("{base}..{end_ref}");
    let output = run_git_command(repo_path, &["diff", "--numstat", &range]).await?;
    let mut entries = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            entries.push(FileChangeEntry {
                path: parts[2].to_string(),
                additions: parts[0].parse().unwrap_or(0),
                deletions: parts[1].parse().unwrap_or(0),
            });
        }
    }
    Ok(entries)
}

#[cfg(not(feature = "local_fs"))]
pub async fn get_branch_diff_entries(_repo_path: &Path) -> Result<Vec<FileChangeEntry>> {
    Err(anyhow!("Not supported on wasm"))
}

/// Pushes the given branch to origin, setting upstream tracking if not already configured.
/// `path_env` is forwarded so the LFS `pre-push` hook can find `git-lfs`.
#[cfg(feature = "local_fs")]
pub async fn run_push(repo_path: &Path, branch: &str, path_env: Option<&str>) -> Result<String> {
    run_git_command_with_env(
        repo_path,
        &["push", "--set-upstream", "origin", branch],
        path_env,
    )
    .await
}

#[cfg(not(feature = "local_fs"))]
pub async fn run_push(_repo_path: &Path, _branch: &str, _path_env: Option<&str>) -> Result<String> {
    Err(anyhow!("Not supported on wasm"))
}

// ── gh CLI helpers ───────────────────────────────────────────────────────────

/// PR information returned by `gh pr view`.
#[derive(Debug, Clone)]
pub struct PrInfo {
    pub number: u64,
    pub url: String,
}

/// Runs a `gh` CLI command and returns stdout on success. `path_env`, when
/// `Some`, is set as the child's `PATH` so a Homebrew-installed `gh` is
/// findable from macOS GUI launches (launchd's minimal `PATH` excludes it).
#[cfg(feature = "local_fs")]
async fn run_gh_command(repo_path: &Path, args: &[&str], path_env: Option<&str>) -> Result<String> {
    use command::r#async::Command;
    use command::Stdio;

    log::debug!(
        "[GIT OPERATION] git.rs run_gh_command gh {}",
        args.join(" ")
    );

    let mut cmd = Command::new("gh");
    cmd.args(args)
        .current_dir(repo_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .kill_on_drop(true);
    if let Some(path_env) = path_env {
        cmd.env("PATH", path_env);
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| anyhow!("Failed to execute gh command: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(anyhow!("gh command failed: {stderr}"))
    }
}

/// Looks up the PR for the current branch via `gh pr view`.
/// Returns `Ok(None)` if there is simply no PR for this branch.
/// Returns `Err` for real failures (auth, network, gh not installed).
#[cfg(feature = "local_fs")]
pub async fn get_pr_for_branch(repo_path: &Path, path_env: Option<&str>) -> Result<Option<PrInfo>> {
    match run_gh_command(repo_path, &["pr", "view", "--json", "number,url"], path_env).await {
        Ok(stdout) => {
            let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
                .map_err(|e| anyhow!("Failed to parse gh output: {e}"))?;
            let number = parsed["number"]
                .as_u64()
                .ok_or_else(|| anyhow!("Missing 'number' in gh output"))?;
            let url = parsed["url"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing 'url' in gh output"))?
                .to_string();
            Ok(Some(PrInfo { number, url }))
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("no pull requests found") {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

#[cfg(not(feature = "local_fs"))]
pub async fn get_pr_for_branch(
    _repo_path: &Path,
    _path_env: Option<&str>,
) -> Result<Option<PrInfo>> {
    Err(anyhow!("Not supported on wasm"))
}

/// PR-ready diff (default branch vs `origin/<current>` or HEAD),
/// truncated for AI token limits.
#[cfg(feature = "local_fs")]
pub async fn get_diff_for_pr(repo_path: &Path) -> Result<String> {
    let base = detect_main_branch(repo_path).await?;
    let base = base.trim();
    let current = detect_current_branch(repo_path).await?;
    let remote_ref = format!("origin/{current}");

    let end_ref = if run_git_command(repo_path, &["rev-parse", "--verify", &remote_ref])
        .await
        .is_ok()
    {
        remote_ref
    } else {
        "HEAD".to_string()
    };

    let range = format!("{base}..{end_ref}");
    let mut diff = run_git_command(repo_path, &["diff", &range]).await?;
    if diff.len() > MAX_DIFF_CHARS_FOR_AI {
        diff = format!(
            "{}\n... (diff truncated)",
            truncate_on_char_boundary(&diff, MAX_DIFF_CHARS_FOR_AI)
        );
    }
    Ok(diff)
}

#[cfg(not(feature = "local_fs"))]
pub async fn get_diff_for_pr(_repo_path: &Path) -> Result<String> {
    Err(anyhow!("Not supported on wasm"))
}

/// Commit subject lines on the current branch since the default branch.
#[cfg(feature = "local_fs")]
pub async fn get_branch_commit_messages(repo_path: &Path) -> Result<Vec<String>> {
    let base = detect_main_branch(repo_path).await?;
    let base = base.trim();
    let range = format!("{base}..HEAD");
    let output = run_git_command(repo_path, &["log", &range, "--format=%s"]).await?;
    Ok(output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect())
}

#[cfg(not(feature = "local_fs"))]
pub async fn get_branch_commit_messages(_repo_path: &Path) -> Result<Vec<String>> {
    Err(anyhow!("Not supported on wasm"))
}

/// Creates a PR for the current branch (must already be pushed). Falls back
/// to `--fill` when title/body are `None`. Always targets the detected
/// default branch.
#[cfg(feature = "local_fs")]
pub async fn create_pr(
    repo_path: &Path,
    title: Option<&str>,
    body: Option<&str>,
    path_env: Option<&str>,
) -> Result<PrInfo> {
    let base = detect_main_branch(repo_path).await?;
    let base = base.trim();
    let base = base.strip_prefix("origin/").unwrap_or(base);
    let sanitized_title;
    let args: Vec<&str> = match (title, body) {
        (Some(t), Some(b)) => {
            sanitized_title = sanitize_pr_title(t);
            vec![
                "pr",
                "create",
                "--base",
                base,
                "--title",
                &sanitized_title,
                "--body",
                b,
            ]
        }
        _ => vec!["pr", "create", "--base", base, "--fill"],
    };
    let stdout = run_gh_command(repo_path, &args, path_env).await?;
    // `gh pr create` prints the PR URL on success.
    let url = stdout.trim().to_string();
    // Extract PR number from the URL (e.g. https://github.com/owner/repo/pull/123)
    let number = url
        .rsplit('/')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| anyhow!("Could not parse PR number from URL: {url}"))?;
    Ok(PrInfo { number, url })
}

/// Trims an AI-generated PR title to a single line and caps its length.
#[cfg(feature = "local_fs")]
fn sanitize_pr_title(raw: &str) -> String {
    let first_line = raw.lines().next().unwrap_or("").trim();
    truncate_on_char_boundary(first_line, MAX_PR_TITLE_BYTES).to_string()
}

#[cfg(not(feature = "local_fs"))]
pub async fn create_pr(
    _repo_path: &Path,
    _title: Option<&str>,
    _body: Option<&str>,
    _path_env: Option<&str>,
) -> Result<PrInfo> {
    Err(anyhow!("Not supported on wasm"))
}

/// A single branch entry returned by [`get_all_branches`].
#[derive(Clone, Debug, PartialEq)]
pub struct BranchEntry {
    pub name: String,
    pub is_main: bool,
}

/// Gets git branches, sorted by commit date (most recent first).
/// Defaults to the most recent 100 branches for performance.
pub async fn get_all_branches(
    repo_path: &Path,
    max_branch_count: Option<usize>,
    include_remotes: bool,
) -> Result<Vec<BranchEntry>> {
    let main_branch = match detect_main_branch(repo_path).await {
        Ok(branch) => branch,
        Err(err) => {
            log::warn!("Failed to detect main branch: {err}");
            "origin/main".to_string()
        }
    };
    fetch_branch_list_with_main(repo_path, &main_branch, max_branch_count, include_remotes).await
}

/// Like [`get_all_branches`] but with a pre-known main branch, skipping
/// [`detect_main_branch`].
///
/// Use this when the main branch is already cached from a previous call to avoid
/// the up-to-6 sequential subprocess calls that detection may require.
pub async fn get_all_branches_with_known_main(
    repo_path: &Path,
    main_branch: &str,
    max_branch_count: Option<usize>,
    include_remotes: bool,
) -> Result<Vec<BranchEntry>> {
    fetch_branch_list_with_main(repo_path, main_branch, max_branch_count, include_remotes).await
}

/// Shared implementation for [`get_all_branches`] and
/// [`get_all_branches_with_known_main`]. Runs `git for-each-ref` and
/// marks each branch as main or not based on the supplied `main_branch` string.
async fn fetch_branch_list_with_main(
    repo_path: &Path,
    main_branch: &str,
    max_branch_count: Option<usize>,
    include_remotes: bool,
) -> Result<Vec<BranchEntry>> {
    let count_arg = format!("--count={}", max_branch_count.unwrap_or(100));

    let mut args = vec![
        "for-each-ref",
        count_arg.as_str(),
        "--sort=-committerdate",
        "--format=%(refname:short)",
        "refs/heads",
    ];

    if include_remotes {
        args.push("refs/remotes");
    }
    log::debug!(
        "[GIT OPERATION] git.rs fetch_branch_list_with_main git {}",
        args.join(" ")
    );
    let output = run_git_command(repo_path, args.as_slice()).await?;

    let mut branches = Vec::new();

    for branch in output.lines() {
        let branch = branch.trim();
        if branch.is_empty() {
            continue;
        }

        // Skip HEAD pointer and detached HEAD states
        if branch.contains("HEAD") || branch.starts_with('(') {
            continue;
        }

        let is_main = branch == main_branch || branch == main_branch.trim_start_matches("origin/");
        branches.push(BranchEntry {
            name: branch.to_string(),
            is_main,
        });
    }

    // Remove duplicates while preserving order (most recent first)
    let mut seen = std::collections::HashSet::new();
    branches.retain(|entry| seen.insert(entry.name.clone()));

    if branches.is_empty() {
        safe_warn!(
            safe: ("Code Review: get_all_branches returned empty list"),
            full: ("Code Review: get_all_branches returned empty list for repo: {:?}", repo_path)
        );
    }

    Ok(branches)
}

/// Returns an iterator over `branches` with main branches first,
/// then the rest in their existing order.
pub fn sort_branches_main_first(branches: &[BranchEntry]) -> impl Iterator<Item = &BranchEntry> {
    branches
        .iter()
        .filter(|entry| entry.is_main)
        .chain(branches.iter().filter(|entry| !entry.is_main))
}

/// Represents a parsed unified diff header.
/// Format: `@@ -old_start,old_count +new_start,new_count @@ [optional context]`
#[derive(Clone, Debug, PartialEq)]
pub struct UnifiedDiffHeader {
    pub old_start_line: usize,
    pub old_line_count: usize,
    pub new_start_line: usize,
    pub new_line_count: usize,
}

/// Parses a range string like "1,5" or "1" into (start, count).
pub(crate) fn parse_range(range_str: &str) -> Result<(usize, usize)> {
    if let Some(comma_pos) = range_str.find(',') {
        let start: usize = range_str[..comma_pos]
            .parse()
            .map_err(|_| anyhow!("Invalid range start: {range_str}"))?;
        let count: usize = range_str[comma_pos + 1..]
            .parse()
            .map_err(|_| anyhow!("Invalid range count: {range_str}"))?;
        Ok((start, count))
    } else {
        let start: usize = range_str
            .parse()
            .map_err(|_| anyhow!("Invalid range: {range_str}"))?;
        Ok((start, 1))
    }
}

/// Parses a unified diff header line.
/// Format: `@@ -old_start,old_count +new_start,new_count @@ [optional context]`
pub(crate) fn parse_unified_diff_header(header_line: &str) -> Result<UnifiedDiffHeader> {
    if !header_line.starts_with("@@") {
        return Err(anyhow!("Invalid unified diff header: {header_line}"));
    }

    // Split by whitespace and take only the first 3 tokens to ignore optional context
    let header_parts: Vec<&str> = header_line.split_whitespace().take(3).collect();
    if header_parts.len() < 3 {
        return Err(anyhow!("Invalid unified diff header format: {header_line}"));
    }

    let old_range = &header_parts[1][1..]; // Remove the '-'
    let new_range = &header_parts[2][1..]; // Remove the '+'

    let (old_start_line, old_line_count) = parse_range(old_range)?;
    let (new_start_line, new_line_count) = parse_range(new_range)?;

    Ok(UnifiedDiffHeader {
        old_start_line,
        old_line_count,
        new_start_line,
        new_line_count,
    })
}

/// Counts newlines in a file, returning 0 for binary or oversized files.
#[cfg(feature = "local_fs")]
fn count_lines_if_text_file(path: &Path) -> u32 {
    const MAX_FILE_SIZE: u64 = 20_000_000;
    const BINARY_CHECK_SIZE: usize = 1024;

    let Ok(metadata) = std::fs::metadata(path) else {
        return 0;
    };
    if metadata.len() > MAX_FILE_SIZE || !metadata.is_file() {
        return 0;
    }
    let Ok(content) = std::fs::read(path) else {
        return 0;
    };
    let check_len = content.len().min(BINARY_CHECK_SIZE);
    if warp_util::file_type::is_buffer_binary(&content[..check_len]) {
        return 0;
    }
    bytecount::count(&content, b'\n') as u32
}
