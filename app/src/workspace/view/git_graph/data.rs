//! Data layer for the Git Graph: commit data types, parsing of `git log`
//! output (pure functions), and async data loading.
//!
//! All fetching goes through [`warp_util::git::run_git_command`] (which shells
//! out to `git`); there is no dependency on `git2`/`gix`. Parsing is kept
//! separate from IO: the `parse_*` functions are pure and unit-testable in
//! isolation, while `load_*` are thin wrappers that just assemble the command
//! and invoke the parser.

#[cfg(not(target_family = "wasm"))]
use std::path::{Path, PathBuf};

#[cfg(not(target_family = "wasm"))]
use anyhow::Result;

/// Field separator within a `git log --pretty=format` record (ASCII Unit
/// Separator). A control character is used instead of a printable one so that
/// ordinary characters in subject / ref names cannot corrupt parsing.
const UNIT_SEP: char = '\u{1f}';
/// Separator between commit records (ASCII Record Separator).
const RECORD_SEP: char = '\u{1e}';

/// Format string passed to `git log`. The field order matches
/// [`parse_commit_record`] exactly:
/// hash / parents / author name / author email / author time / decorate / subject.
/// `%x1f` and `%x1e` are git's escape notation for the two separator bytes above.
#[cfg(not(target_family = "wasm"))]
const LOG_FORMAT: &str = "--pretty=format:%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e";

/// A single commit node, carrying everything needed to render one graph row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitNode {
    /// Full commit hash.
    pub hash: String,
    /// Short hash for display (first 7 characters).
    pub short_hash: String,
    /// Full hashes of the parent commits: 0 means a root, 2 or more a merge.
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// Author time (Unix seconds).
    pub author_time: i64,
    /// First line of the commit message.
    pub subject: String,
    /// Ref labels pointing at this commit (branch / remote branch / tag / HEAD).
    pub refs: Vec<RefLabel>,
}

/// Kind of ref label; determines the rendering style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefKind {
    /// The currently checked-out position (`HEAD`).
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
}

/// A single ref label pointing at a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefLabel {
    pub kind: RefKind,
    /// Display name (with prefixes like `refs/heads/` stripped).
    pub name: String,
}

/// Parse the entire output of `git log` (in [`LOG_FORMAT`]) into a commit list.
pub(crate) fn parse_commit_log(stdout: &str) -> Vec<CommitNode> {
    stdout
        .split(RECORD_SEP)
        .filter_map(|record| {
            // git joins records with newlines, so trim leading/trailing
            // whitespace and newlines before parsing.
            let record = record.trim_matches(|c: char| c == '\n' || c == '\r');
            if record.is_empty() {
                return None;
            }
            parse_commit_record(record)
        })
        .collect()
}

/// Parse a single commit record. Returns `None` when fields are missing
/// (skipping the record rather than panicking).
fn parse_commit_record(record: &str) -> Option<CommitNode> {
    // splitn(7) ensures the final subject field is kept intact even if it
    // contains the separator.
    let mut fields = record.splitn(7, UNIT_SEP);
    let hash = fields.next()?.to_string();
    let parents_raw = fields.next()?;
    let author_name = fields.next()?.to_string();
    let author_email = fields.next()?.to_string();
    let author_time = fields.next()?.trim().parse::<i64>().ok()?;
    let decorate = fields.next()?;
    let subject = fields.next().unwrap_or("").to_string();

    let parents = parents_raw
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let short_hash = hash.chars().take(7).collect::<String>();
    let refs = parse_decorate(decorate);

    Some(CommitNode {
        hash,
        short_hash,
        parents,
        author_name,
        author_email,
        author_time,
        subject,
        refs,
    })
}

/// Parse the `%D` decorate string (from `--decorate=full`) into a list of ref labels.
///
/// Input looks like `HEAD -> refs/heads/main, refs/remotes/origin/main, refs/tags/v1`.
/// Full mode is used so local branches / remote branches / tags can be told
/// apart reliably (short mode cannot distinguish a local `feature/x` from a
/// remote `origin/x`).
pub(crate) fn parse_decorate(decorate: &str) -> Vec<RefLabel> {
    let decorate = decorate.trim();
    if decorate.is_empty() {
        return Vec::new();
    }
    decorate
        .split(',')
        .filter_map(|raw| {
            let token = raw.trim();
            if token.is_empty() {
                return None;
            }
            // "HEAD -> refs/heads/main": the branch HEAD currently points at.
            if let Some(branch) = token.strip_prefix("HEAD -> ") {
                let name = branch.strip_prefix("refs/heads/").unwrap_or(branch);
                return Some(RefLabel {
                    kind: RefKind::Head,
                    name: name.to_string(),
                });
            }
            // Detached HEAD.
            if token == "HEAD" {
                return Some(RefLabel {
                    kind: RefKind::Head,
                    name: "HEAD".to_string(),
                });
            }
            // git --decorate=full prefixes tag refs with "tag: " to set them
            // apart from branches, e.g. "tag: refs/tags/v1.0"; strip both the
            // "tag: " marker and the "refs/tags/" ref prefix to get the name.
            if let Some(rest) = token.strip_prefix("tag: ") {
                let name = rest.strip_prefix("refs/tags/").unwrap_or(rest);
                return Some(RefLabel {
                    kind: RefKind::Tag,
                    name: name.to_string(),
                });
            }
            if let Some(remote) = token.strip_prefix("refs/remotes/") {
                // Hide a remote's symbolic HEAD (e.g. origin/HEAD); it is
                // meaningless for browsing history.
                if remote.ends_with("/HEAD") {
                    return None;
                }
                return Some(RefLabel {
                    kind: RefKind::RemoteBranch,
                    name: remote.to_string(),
                });
            }
            if let Some(local) = token.strip_prefix("refs/heads/") {
                return Some(RefLabel {
                    kind: RefKind::LocalBranch,
                    name: local.to_string(),
                });
            }
            // Ignore other unknown decorations (e.g. grafted / replaced).
            None
        })
        .collect()
}

/// Load the commit graph for a repository.
///
/// `branch_refs` controls the graph's coverage: `None` uses `--all` (every
/// ref); `Some(refs)` covers only the given branch refs (e.g. `refs/heads/main`,
/// `refs/remotes/origin/dev`), and an empty slice means the user deselected all
/// branches, so an empty graph is returned. `limit`/`skip` drive pagination.
/// `--date-order` keeps the layout stable, and `--decorate=full` lets
/// [`parse_decorate`] classify `%D` reliably.
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_commit_graph(
    repo_root: &Path,
    branch_refs: Option<&[String]>,
    limit: usize,
    skip: usize,
) -> Result<Vec<CommitNode>> {
    let n = limit.to_string();
    let skip_s = skip.to_string();
    let mut args: Vec<&str> = vec![
        "log",
        "--date-order",
        "--decorate=full",
        "--no-color",
        "-n",
        &n,
        "--skip",
        &skip_s,
        LOG_FORMAT,
    ];
    // The selected branch refs follow the options as revisions; fall back to
    // --all when None.
    match branch_refs {
        None => args.push("--all"),
        Some(refs) => {
            if refs.is_empty() {
                return Ok(Vec::new());
            }
            args.extend(refs.iter().map(String::as_str));
        }
    }
    let stdout = warp_util::git::run_git_command(repo_root, &args).await?;
    Ok(parse_commit_log(&stdout))
}

/// How long the manual-refresh `git fetch` may run before we give up. A private
/// repo whose remote is unreachable (e.g. no VPN) would otherwise hang the fetch
/// on the TCP connect indefinitely; after this the future is dropped — which
/// kills the git child via `kill_on_drop` — and the caller falls back to a local
/// reload while surfacing a "couldn't reach remote" banner.
#[cfg(not(target_family = "wasm"))]
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Fetches from the configured remotes and prunes deleted remote-tracking refs
/// (`git fetch --prune`). Used by the manual refresh button so the graph picks
/// up new remote commits and drops branches that were deleted on the remote.
///
/// A repo with no remote configured returns `Ok` (nothing to fetch is not a
/// failure). A genuine fetch failure — unreachable remote, auth rejection, or
/// the [`FETCH_TIMEOUT`] elapsing — returns `Err`; callers treat that as
/// fail-soft (still reload the local graph) but surface it to the user.
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn fetch_remotes(repo_root: &Path) -> Result<()> {
    use futures::future::Either;
    use warpui::r#async::Timer;

    // A repo with no remote has nothing to fetch — don't run `git fetch` (which
    // would error) so we never surface a spurious "couldn't reach remote" banner
    // on a purely local repo.
    let remotes = warp_util::git::run_git_command(repo_root, &["remote"])
        .await
        .unwrap_or_default();
    if remotes.trim().is_empty() {
        return Ok(());
    }

    // Bound the fetch so an unreachable remote can't hang the refresh forever.
    let fetch = warp_util::git::run_git_command(repo_root, &["fetch", "--prune"]);
    let timeout = Timer::after(FETCH_TIMEOUT);
    futures::pin_mut!(fetch);
    futures::pin_mut!(timeout);
    match futures::future::select(fetch, timeout).await {
        Either::Left((result, _)) => result.map(|_| ()),
        Either::Right(_) => Err(anyhow::anyhow!(
            "git fetch timed out after {}s",
            FETCH_TIMEOUT.as_secs()
        )),
    }
}

/// A branch ref usable for filtering the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchRef {
    /// Display name (with the `refs/heads/` or `refs/remotes/` prefix stripped).
    pub display_name: String,
    /// Full ref passed to `git log` (e.g. `refs/heads/main`, `refs/remotes/origin/main`).
    pub ref_name: String,
    /// Whether this is a local or remote branch (drives grouping/styling).
    pub kind: RefKind,
}

/// Load a repository's local + remote branch list (for the branch-filter dropdown).
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_branches(repo_root: &Path) -> Result<Vec<BranchRef>> {
    let args = [
        "for-each-ref",
        "--format=%(refname)",
        "refs/heads",
        "refs/remotes",
    ];
    let stdout = warp_util::git::run_git_command(repo_root, &args).await?;
    Ok(parse_branch_refs(&stdout))
}

/// Parse the output of `git for-each-ref --format=%(refname)` into a branch list.
///
/// Each line is a full ref. Remote symbolic HEADs (`refs/remotes/*/HEAD`, which
/// are meaningless for browsing) are filtered out. Local branches come first,
/// then remote branches, each sorted by name to keep the dropdown order stable.
pub(crate) fn parse_branch_refs(stdout: &str) -> Vec<BranchRef> {
    let mut locals = Vec::new();
    let mut remotes = Vec::new();
    for line in stdout.lines() {
        let full = line.trim();
        if let Some(name) = full.strip_prefix("refs/heads/") {
            locals.push(BranchRef {
                display_name: name.to_string(),
                ref_name: full.to_string(),
                kind: RefKind::LocalBranch,
            });
        } else if let Some(name) = full.strip_prefix("refs/remotes/") {
            if name.ends_with("/HEAD") {
                continue;
            }
            remotes.push(BranchRef {
                display_name: name.to_string(),
                ref_name: full.to_string(),
                kind: RefKind::RemoteBranch,
            });
        }
    }
    locals.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    remotes.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    locals.extend(remotes);
    locals
}

/// Discover all git repository roots related to `anchor`, returned in display
/// order (deduplicated):
/// 1. The repository the anchor itself belongs to: probed upward with
///    `git rev-parse --show-toplevel`. The anchor may be a subdirectory of a
///    repository (e.g. the terminal has `cd`'d into `repo/crates`); this step
///    preserves the behavior of "viewing the parent repo's history even from a
///    subdirectory" and places it first in the list (closest to where the user
///    currently is).
/// 2. Repositories in subdirectories at levels 1..=`depth`: see
///    [`scan_subdir_repos`].
///
/// Used for the case of "several independent git projects sitting under one
/// directory" (e.g. using `~/Projects` as the working directory).
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn discover_repositories(anchor: &Path, depth: usize) -> Vec<PathBuf> {
    let mut repos: Vec<PathBuf> = Vec::new();

    // The anchor's own repository (probed upward). On failure (not inside any
    // repository) skip silently rather than erroring.
    if let Ok(stdout) =
        warp_util::git::run_git_command(anchor, &["rev-parse", "--show-toplevel"]).await
    {
        let toplevel = stdout.trim();
        if !toplevel.is_empty() {
            repos.push(PathBuf::from(toplevel));
        }
    }

    // Independent repositories in subdirectories (deduplicated against the
    // anchor's own repository).
    for repo in scan_subdir_repos(anchor, depth) {
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }

    repos
}

/// Scan the subdirectories of `anchor` at levels 1..=`depth`, returning those
/// that carry a `.git` marker (repository roots), sorted by path.
///
/// Semantics: `anchor` itself is level 0 and its direct children are level 1.
/// When `depth==0` no subdirectories are scanned. Once a repository root is hit,
/// its interior is **not** descended into — this avoids treating its submodules /
/// nested repositories as sibling independent projects.
#[cfg(not(target_family = "wasm"))]
fn scan_subdir_repos(anchor: &Path, depth: usize) -> Vec<PathBuf> {
    use std::collections::VecDeque;

    let mut found: Vec<PathBuf> = Vec::new();
    // BFS queue: (directory, the directory's level).
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((anchor.to_path_buf(), 0));

    while let Some((dir, level)) = queue.pop_front() {
        // Depth limit reached: do not expand this level's children any further.
        if level >= depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // Only look at directories (ignore plain files; `.git` may be either
            // a directory or a file, so it is detected via `exists`).
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            if path.join(".git").exists() {
                // A repository root: record it and do not enqueue its children
                // (do not descend into the repository).
                found.push(path);
            } else {
                // A plain directory: keep scanning into the next level.
                queue.push_back((path, level + 1));
            }
        }
    }

    // read_dir order is filesystem-dependent; sorting keeps the list stable
    // (deterministic UI dropdown order, reproducible tests).
    found.sort();
    found
}

/// A single changed file in a commit, with its insertion/deletion counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangedFile {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
}

/// Details of the selected commit: committer info, full commit message, and the
/// list of changed files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitDetail {
    pub committer_name: String,
    pub committer_time: i64,
    /// Full commit message (`%B`, including subject and body).
    pub message: String,
    pub files: Vec<ChangedFile>,
}

/// Load the details of a single commit.
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_commit_detail(repo_root: &Path, hash: &str) -> Result<CommitDetail> {
    // `%x1e` separates the format header from the `--numstat` lines that follow.
    let args = [
        "show",
        "--numstat",
        "--no-color",
        "--format=%cn%x1f%ct%x1f%B%x1e",
        hash,
    ];
    let stdout = warp_util::git::run_git_command(repo_root, &args).await?;
    Ok(parse_commit_detail(&stdout))
}

/// Parse the output of `git show --numstat --format=%cn%x1f%ct%x1f%B%x1e`.
pub(crate) fn parse_commit_detail(stdout: &str) -> CommitDetail {
    let (header, numstat) = stdout.split_once(RECORD_SEP).unwrap_or((stdout, ""));
    let mut fields = header.splitn(3, UNIT_SEP);
    let committer_name = fields.next().unwrap_or("").to_string();
    let committer_time = fields
        .next()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let message = fields.next().unwrap_or("").trim().to_string();

    CommitDetail {
        committer_name,
        committer_time,
        message,
        files: parse_numstat(numstat),
    }
}

/// Parse `--numstat` output (each line is `additions\tdeletions\tpath`; binary
/// files use `-`).
fn parse_numstat(stdout: &str) -> Vec<ChangedFile> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut parts = line.splitn(3, '\t');
            let additions = parts.next()?;
            let deletions = parts.next()?;
            let path = parts.next()?.to_string();
            Some(ChangedFile {
                path,
                // Binary files report "-" in these columns; treat as 0.
                additions: additions.parse::<u32>().unwrap_or(0),
                deletions: deletions.parse::<u32>().unwrap_or(0),
            })
        })
        .collect()
}

/// The change a selected commit made to one file (`commit~1..commit`, i.e. "what
/// this commit itself changed"): the full file content at the parent version (as
/// the diff base) plus the parsed unified diff hunks.
/// Used for "click a changed file in the commit detail -> open a read-only diff
/// pane in the main area".
#[cfg(not(target_family = "wasm"))]
#[derive(Debug, Clone)]
pub(crate) struct CommitFileDiff {
    /// Full content of the file at the parent commit; an empty string for an
    /// added file or a root commit with no parent version (in which case the
    /// diff is entirely added lines).
    pub base_content: String,
    /// The commit's unified diff hunks for this file (reusing the code review
    /// parser and types).
    pub hunks: Vec<crate::code_review::diff_state::DiffHunk>,
}

/// Load a commit's change to a single file. `path` is a repository-relative path.
///
/// The fetch strategy handles plain / added / deleted / root / merge commits:
/// - base content: `git show <hash>~1:<path>`; if unavailable (an added file or
///   a root commit with no parent version) it is treated as an empty string.
/// - diff: prefer `git diff <hash>~1 <hash> -- <path>` (this yields the normal
///   "compared against the first parent" hunks even for merge commits, avoiding
///   the unparseable combined-diff `@@@` that `git show` emits for merges); when
///   `<hash>~1` does not exist (a root commit), fall back to
///   `git show <hash> --format= -- <path>` (showing the whole file as added).
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_file_diff_at_commit(
    repo_root: &Path,
    hash: &str,
    path: &str,
) -> Result<CommitFileDiff> {
    use crate::code_review::diff_state::LocalDiffStateModel;

    let parent = format!("{hash}~1");

    // base: full content of the parent version; for an added file / a root
    // commit with no parent version git errors out, so treat it as empty.
    let base_spec = format!("{parent}:{path}");
    let base_content = warp_util::git::run_git_command(repo_root, &["show", base_spec.as_str()])
        .await
        .unwrap_or_default();

    // diff: first take the normal two-way diff "compared against the first
    // parent"; for a root commit with no parent, fall back to the whole file as
    // added.
    let diff_output = match warp_util::git::run_git_command(
        repo_root,
        &["diff", "--no-color", parent.as_str(), hash, "--", path],
    )
    .await
    {
        Ok(out) => out,
        Err(_) => {
            warp_util::git::run_git_command(
                repo_root,
                &["show", "--no-color", "--format=", hash, "--", path],
            )
            .await?
        }
    };

    let hunks = LocalDiffStateModel::parse_diff_hunks(&diff_output)?;
    Ok(CommitFileDiff {
        base_content,
        hunks,
    })
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
