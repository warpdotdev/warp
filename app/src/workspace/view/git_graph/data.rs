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

#[cfg(not(target_family = "wasm"))]
use crate::code::commit_diff_view::DiffPreview;

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
    /// Short hash for display (first 8 characters).
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
    /// A stash entry (`stash@{n}`). Synthetic: injected from `git stash list`
    /// rather than parsed from a `git log` decorate string.
    Stash,
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
    let short_hash = hash.chars().take(8).collect::<String>();
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
                // A remote's symbolic HEAD (e.g. origin/HEAD) is shown like any
                // other remote branch so the remote's default branch is visible
                // at a glance. (The branch *filter* list still omits it — see
                // `parse_branch_refs` — since it isn't independently selectable.)
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
        None => {
            // `--all` walks every ref under refs/ — which includes `refs/stash`,
            // surfacing the stash commit *and* its index/untracked auxiliary
            // commits as stray graph nodes. Exclude it; stashes are loaded and
            // injected separately (see `load_stashes`) as clean single-parent
            // nodes. `--exclude` must precede the `--all` it filters.
            args.push("--exclude=refs/stash");
            args.push("--all");
        }
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

/// `git stash list` format string; the field order matches [`parse_stash_record`]:
/// selector (`%gd`, e.g. `stash@{0}`) / hash / parents / author name / email /
/// time / subject.
#[cfg(not(target_family = "wasm"))]
const STASH_FORMAT: &str = "--format=%gd%x1f%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1e";

/// True when this node is a synthetic stash entry (carries a [`RefKind::Stash`]
/// label) rather than a real `git log` commit. Used to keep stashes out of the
/// pagination skip count.
pub(crate) fn is_stash_node(commit: &CommitNode) -> bool {
    commit.refs.iter().any(|r| r.kind == RefKind::Stash)
}

/// Parse `git stash list` output (in [`STASH_FORMAT`]) into synthetic commit
/// nodes. Each stash keeps only its base (first) parent — the index/untracked
/// auxiliary parents are dropped so they don't show up as stray nodes that
/// pollute the lane layout — and is tagged with a [`RefKind::Stash`] label.
pub(crate) fn parse_stash_list(stdout: &str) -> Vec<CommitNode> {
    stdout
        .split(RECORD_SEP)
        .filter_map(|record| {
            let record = record.trim_matches(|c: char| c == '\n' || c == '\r');
            if record.is_empty() {
                return None;
            }
            parse_stash_record(record)
        })
        .collect()
}

/// Parse a single `git stash list` record into a synthetic commit node.
fn parse_stash_record(record: &str) -> Option<CommitNode> {
    let mut fields = record.splitn(7, UNIT_SEP);
    let selector = fields.next()?.trim().to_string();
    let hash = fields.next()?.to_string();
    let parents_raw = fields.next()?;
    let author_name = fields.next()?.to_string();
    let author_email = fields.next()?.to_string();
    let author_time = fields.next()?.trim().parse::<i64>().ok()?;
    let subject = fields.next().unwrap_or("").to_string();

    // Only the base (first) parent matters for the graph; the index/untracked
    // parents are stash internals and would otherwise draw bogus nodes.
    let base = parents_raw.split_whitespace().next()?.to_string();
    let short_hash = hash.chars().take(8).collect::<String>();
    Some(CommitNode {
        hash,
        short_hash,
        parents: vec![base],
        author_name,
        author_email,
        author_time,
        subject,
        refs: vec![RefLabel {
            kind: RefKind::Stash,
            name: selector,
        }],
    })
}

/// Merge stash nodes into a commit list (both already newest→oldest) by author
/// time descending, so each stash appears near where it was created. Stable on
/// ties: a stash sorts ahead of a commit with the same time.
pub(crate) fn merge_stashes(commits: Vec<CommitNode>, stashes: Vec<CommitNode>) -> Vec<CommitNode> {
    if stashes.is_empty() {
        return commits;
    }
    let mut merged = Vec::with_capacity(commits.len() + stashes.len());
    let mut commits = commits.into_iter().peekable();
    let mut stashes = stashes.into_iter().peekable();
    loop {
        match (stashes.peek(), commits.peek()) {
            (Some(s), Some(c)) => {
                if s.author_time >= c.author_time {
                    merged.push(stashes.next().unwrap());
                } else {
                    merged.push(commits.next().unwrap());
                }
            }
            (Some(_), None) => merged.push(stashes.next().unwrap()),
            (None, Some(_)) => merged.push(commits.next().unwrap()),
            (None, None) => break,
        }
    }
    merged
}

/// Loads the repo's stashes as synthetic commit nodes (see [`parse_stash_list`]).
/// A repo with no stashes returns an empty list.
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_stashes(repo_root: &Path) -> Result<Vec<CommitNode>> {
    let stdout =
        warp_util::git::run_git_command(repo_root, &["stash", "list", STASH_FORMAT]).await?;
    Ok(parse_stash_list(&stdout))
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

/// Count changed files from `git status --porcelain` output: it prints one line
/// per changed file (staged, unstaged, or untracked), so the number of
/// uncommitted changes is the count of non-empty lines.
pub(crate) fn parse_status_count(stdout: &str) -> usize {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
}

/// Number of uncommitted changed files in the working tree (staged + unstaged +
/// untracked). `0` means a clean tree.
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_working_tree_status(repo_root: &Path) -> Result<usize> {
    let stdout = warp_util::git::run_git_command(repo_root, &["status", "--porcelain"]).await?;
    Ok(parse_status_count(&stdout))
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
    pub committer_email: String,
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
        "--format=%cn%x1f%ce%x1f%ct%x1f%B%x1e",
        hash,
    ];
    let stdout = warp_util::git::run_git_command(repo_root, &args).await?;
    Ok(parse_commit_detail(&stdout))
}

/// Parse the output of `git show --numstat --format=%cn%x1f%ce%x1f%ct%x1f%B%x1e`.
pub(crate) fn parse_commit_detail(stdout: &str) -> CommitDetail {
    let (header, numstat) = stdout.split_once(RECORD_SEP).unwrap_or((stdout, ""));
    let mut fields = header.splitn(4, UNIT_SEP);
    let committer_name = fields.next().unwrap_or("").to_string();
    let committer_email = fields.next().unwrap_or("").to_string();
    let committer_time = fields
        .next()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let message = fields.next().unwrap_or("").trim().to_string();

    CommitDetail {
        committer_name,
        committer_email,
        committer_time,
        message,
        files: parse_numstat(numstat),
    }
}

/// Normalize the `--numstat` path column into the post-rename real path.
///
/// Git compresses renames/moves into one column with two shapes:
///   - braced, sharing a prefix/suffix: `a/{old => new}/c.rs` -> `a/new/c.rs`
///     (either side of `=>` may be empty, e.g. `a/{ => v2}/c.rs`);
///   - unbraced, when nothing is shared: `old => new` -> `new`.
/// Keeping the new path lets the file tree split cleanly on `/` and lets a
/// click open a diff against a path that actually exists in the commit.
fn rename_target_path(raw: &str) -> String {
    const ARROW: &str = " => ";
    if let Some((prefix, rest)) = raw.split_once('{') {
        if let Some((inside, suffix)) = rest.split_once('}') {
            let new = inside.split_once(ARROW).map_or(inside, |(_, new)| new);
            return format!("{prefix}{new}{suffix}");
        }
    }
    if let Some((_, new)) = raw.split_once(ARROW) {
        return new.to_string();
    }
    raw.to_string()
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
            let path = rename_target_path(parts.next()?);
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
    /// How the diff pane should present this file. For `Binary`/`Symlink`,
    /// `base_content` and `hunks` are left empty and the view shows a centered
    /// placeholder instead of a textual diff.
    pub preview: DiffPreview,
}

/// Whether a `git diff` output describes a binary file. For binary files git
/// emits a top-level `Binary files a/x and b/x differ` line (no `@@` hunks)
/// instead of a textual diff; detecting it lets the caller skip the unparseable
/// content and signal the view to show a placeholder.
#[cfg(not(target_family = "wasm"))]
fn diff_is_binary(diff_output: &str) -> bool {
    diff_output
        .lines()
        .any(|line| line.starts_with("Binary files ") && line.ends_with(" differ"))
}

/// git's file mode for a symbolic link. It appears in the diff metadata as
/// `new file mode 120000` (added), `deleted file mode 120000` (removed),
/// `old/new mode 120000` (type change), or trailing the `index ...` line when
/// only the target changed.
#[cfg(not(target_family = "wasm"))]
const GIT_SYMLINK_MODE: &str = "120000";

/// Whether a `git diff` output describes a symbolic link. A symlink's blob
/// content is just the target path, so a textual diff is a lone one-line entry
/// that looks broken; detecting it lets the view show a placeholder naming the
/// target instead. We match the symlink file mode `120000` only where git puts
/// a mode (the `... mode`/`index` metadata lines), checking it is the trailing
/// token so a blob hash that merely ends in those digits can't trip detection.
#[cfg(not(target_family = "wasm"))]
fn diff_is_symlink(diff_output: &str) -> bool {
    diff_output.lines().any(|line| {
        let is_mode_line = line.starts_with("new file mode ")
            || line.starts_with("deleted file mode ")
            || line.starts_with("old mode ")
            || line.starts_with("new mode ")
            || line.starts_with("index ");
        is_mode_line && line.split(' ').next_back() == Some(GIT_SYMLINK_MODE)
    })
}

/// The path a symlink points to, taken from the diff body: the added (`+`)
/// content line for a new/retargeted link, or the removed (`-`) line for a
/// deleted one. Empty if neither is present (shouldn't happen for a real symlink
/// diff). Only meaningful when [`diff_is_symlink`] is true.
#[cfg(not(target_family = "wasm"))]
fn symlink_target(diff_output: &str) -> String {
    // The `+++ `/`--- ` file headers and the `\ No newline` marker are excluded:
    // the former start with `+++`/`---`, the latter starts with `\`.
    let pick = |sign: char, header: &str| {
        diff_output
            .lines()
            .filter(|l| l.starts_with(sign) && !l.starts_with(header))
            .next_back()
            .map(|l| l[1..].to_string())
    };
    pick('+', "+++")
        .or_else(|| pick('-', "---"))
        .unwrap_or_default()
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

    // Symlink: its blob is just the target path, so a textual diff is a lone
    // one-line entry that looks broken — show a placeholder naming the target.
    if diff_is_symlink(&diff_output) {
        return Ok(CommitFileDiff {
            base_content: String::new(),
            hunks: Vec::new(),
            preview: DiffPreview::Symlink {
                target: symlink_target(&diff_output),
            },
        });
    }

    // Binary file: git reports `Binary files ... differ` with no parsable
    // hunks, and the parent revision's bytes are not meaningful as text — skip
    // both and let the view show a placeholder.
    if diff_is_binary(&diff_output) {
        return Ok(CommitFileDiff {
            base_content: String::new(),
            hunks: Vec::new(),
            preview: DiffPreview::Binary,
        });
    }

    let hunks = LocalDiffStateModel::parse_diff_hunks(&diff_output)?;
    Ok(CommitFileDiff {
        base_content,
        hunks,
        preview: DiffPreview::Text,
    })
}

/// Load the working tree's uncommitted changes as a detail (reusing
/// [`CommitDetail`]): tracked changes vs HEAD (`git diff HEAD --numstat`) plus
/// untracked files. `committer`/`message` are left empty — the view renders a
/// dedicated "Uncommitted Changes" header instead.
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_uncommitted_detail(repo_root: &Path) -> Result<CommitDetail> {
    let numstat =
        warp_util::git::run_git_command(repo_root, &["diff", "HEAD", "--numstat", "--no-color"])
            .await
            .unwrap_or_default();
    let mut files = parse_numstat(&numstat);
    // Untracked files aren't part of `git diff HEAD`; list them and diff each
    // against an empty base so its lines count as additions (a new file is all
    // additions, no deletions; binary → 0, like `parse_numstat`).
    let untracked =
        warp_util::git::run_git_command(repo_root, &["ls-files", "--others", "--exclude-standard"])
            .await
            .unwrap_or_default();
    for path in untracked.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let numstat = warp_util::git::run_git_command(
            repo_root,
            &[
                "diff",
                "--no-index",
                "--numstat",
                "--no-color",
                "/dev/null",
                path,
            ],
        )
        .await
        .unwrap_or_default();
        let additions = parse_numstat(&numstat)
            .first()
            .map(|f| f.additions)
            .unwrap_or(0);
        files.push(ChangedFile {
            path: path.to_string(),
            additions,
            deletions: 0,
        });
    }
    Ok(CommitDetail {
        committer_name: String::new(),
        committer_email: String::new(),
        committer_time: 0,
        // Used as the detail's subject line (render_detail_body falls back to the
        // message when there's no commit).
        message: "Uncommitted changes".to_string(),
        files,
    })
}

/// Load the working tree's change to a single file (working vs HEAD), for the
/// uncommitted row's "click a file → diff pane". Mirrors
/// [`load_file_diff_at_commit`] but against the working tree.
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_uncommitted_file_diff(
    repo_root: &Path,
    path: &str,
) -> Result<CommitFileDiff> {
    use crate::code_review::diff_state::LocalDiffStateModel;

    // base: file content at HEAD; empty for an untracked / newly added file.
    let base_spec = format!("HEAD:{path}");
    let base_content = warp_util::git::run_git_command(repo_root, &["show", base_spec.as_str()])
        .await
        .unwrap_or_default();

    // diff: working tree vs HEAD for tracked changes; an untracked file isn't in
    // `git diff HEAD`, so fall back to showing the whole file as added.
    let mut diff_output =
        warp_util::git::run_git_command(repo_root, &["diff", "--no-color", "HEAD", "--", path])
            .await
            .unwrap_or_default();
    if diff_output.trim().is_empty() {
        diff_output = warp_util::git::run_git_command(
            repo_root,
            &["diff", "--no-color", "--no-index", "/dev/null", path],
        )
        .await
        .unwrap_or_default();
    }

    // Symlink / binary: same handling as the committed case above.
    if diff_is_symlink(&diff_output) {
        return Ok(CommitFileDiff {
            base_content: String::new(),
            hunks: Vec::new(),
            preview: DiffPreview::Symlink {
                target: symlink_target(&diff_output),
            },
        });
    }
    if diff_is_binary(&diff_output) {
        return Ok(CommitFileDiff {
            base_content: String::new(),
            hunks: Vec::new(),
            preview: DiffPreview::Binary,
        });
    }

    let hunks = LocalDiffStateModel::parse_diff_hunks(&diff_output)?;
    Ok(CommitFileDiff {
        base_content,
        hunks,
        preview: DiffPreview::Text,
    })
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
