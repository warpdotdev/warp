use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use warp_util::git::run_git_command;

pub const HISTORY_PAGE_SIZE: usize = 50;

const FIELD_SEPARATOR: char = '\u{1f}';
const RECORD_SEPARATOR: char = '\u{1e}';
const HISTORY_FORMAT: &str = "%x1f%H%x1f%P%x1f%an%x1f%at%x1f%s%x1f%D%x1e";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { old_path: String },
    Copied { old_path: String },
    Untracked,
    Conflicted,
}

impl GitChangeKind {
    pub fn status_letter(&self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed { .. } => "R",
            Self::Copied { .. } => "C",
            Self::Untracked => "U",
            Self::Conflicted => "!",
        }
    }

    pub fn previous_path(&self) -> Option<&str> {
        match self {
            Self::Renamed { old_path } | Self::Copied { old_path } => Some(old_path),
            Self::Added | Self::Modified | Self::Deleted | Self::Untracked | Self::Conflicted => {
                None
            }
        }
    }

    pub fn paths_for_action(&self, path: &str) -> Vec<String> {
        match self {
            Self::Renamed { old_path } => vec![old_path.clone(), path.to_string()],
            Self::Added
            | Self::Modified
            | Self::Deleted
            | Self::Copied { .. }
            | Self::Untracked
            | Self::Conflicted => vec![path.to_string()],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileChange {
    pub path: String,
    pub kind: GitChangeKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitRefKind {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRefLabel {
    pub name: String,
    pub kind: GitRefKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitNode {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: i64,
    pub subject: String,
    pub refs: Vec<GitRefLabel>,
}

impl CommitNode {
    pub fn short_hash(&self) -> &str {
        self.hash.get(..7).unwrap_or(&self.hash)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepositorySnapshot {
    pub merge_changes: Vec<FileChange>,
    pub staged_changes: Vec<FileChange>,
    pub changes: Vec<FileChange>,
    pub untracked_changes: Vec<FileChange>,
    pub commits: Vec<CommitNode>,
    pub has_more_history: bool,
    pub has_head: bool,
}

impl RepositorySnapshot {
    pub fn has_changes(&self) -> bool {
        !self.merge_changes.is_empty()
            || !self.staged_changes.is_empty()
            || !self.changes.is_empty()
            || !self.untracked_changes.is_empty()
    }
}

/// Parses `git diff --name-status -z`. Git terminates the status and every path with NUL,
/// including both paths for rename/copy records, so whitespace and newlines remain unambiguous.
pub fn parse_name_status_z(output: &str) -> Vec<FileChange> {
    let fields: Vec<_> = output.split('\0').collect();
    let mut changes = Vec::new();
    let mut index = 0;

    while index < fields.len() {
        let status = fields[index];
        index += 1;
        if status.is_empty() {
            continue;
        }

        let Some(first_path) = fields.get(index).copied() else {
            break;
        };
        index += 1;

        let status_letter = status.as_bytes().first().copied().map(char::from);
        let (path, kind) = match status_letter {
            // Unmerged status pairs such as AA, AU, and DD must be classified before their
            // leading letter is interpreted as an ordinary added/deleted status.
            Some(_) if status.len() == 2 && status.chars().all(|c| "ADU".contains(c)) => {
                (first_path, GitChangeKind::Conflicted)
            }
            Some('R') | Some('C') => {
                let Some(new_path) = fields.get(index).copied() else {
                    break;
                };
                index += 1;
                let kind = if status_letter == Some('R') {
                    GitChangeKind::Renamed {
                        old_path: first_path.to_string(),
                    }
                } else {
                    GitChangeKind::Copied {
                        old_path: first_path.to_string(),
                    }
                };
                (new_path, kind)
            }
            Some('A') => (first_path, GitChangeKind::Added),
            Some('D') => (first_path, GitChangeKind::Deleted),
            Some('U') => (first_path, GitChangeKind::Conflicted),
            Some('M') | Some('T') => (first_path, GitChangeKind::Modified),
            Some(_) | None => continue,
        };

        // `run_git_command` decodes lossily. Dropping replacement-containing paths is safer than
        // presenting a path that a later stage/unstage action could resolve to a different file.
        if !path.is_empty() && !path.contains('\u{fffd}') {
            changes.push(FileChange {
                path: path.to_string(),
                kind,
            });
        }
    }

    changes
}

pub fn parse_untracked_z(output: &str) -> Vec<FileChange> {
    output
        .split('\0')
        .filter(|path| !path.is_empty() && !path.contains('\u{fffd}'))
        .map(|path| FileChange {
            path: path.to_string(),
            kind: GitChangeKind::Untracked,
        })
        .collect()
}

pub fn parse_history(output: &str) -> Vec<CommitNode> {
    output
        .split(RECORD_SEPARATOR)
        .filter_map(|record| {
            let record = record.trim_start_matches(['\r', '\n']);
            let record = record.strip_prefix(FIELD_SEPARATOR)?;
            let mut fields = record.split(FIELD_SEPARATOR);
            let hash = fields.next()?.to_string();
            let parents = fields
                .next()?
                .split_whitespace()
                .map(ToString::to_string)
                .collect();
            let author = fields.next()?.to_string();
            let timestamp = fields.next()?.parse().ok()?;
            let subject = fields.next()?.to_string();
            let refs = parse_refs(fields.next().unwrap_or_default());

            (!hash.is_empty()).then_some(CommitNode {
                hash,
                parents,
                author,
                timestamp,
                subject,
                refs,
            })
        })
        .collect()
}

fn parse_refs(decorations: &str) -> Vec<GitRefLabel> {
    let mut refs = Vec::new();
    for decoration in decorations
        .split(',')
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        if let Some(target) = decoration.strip_prefix("HEAD -> ") {
            refs.push(GitRefLabel {
                name: "HEAD".to_string(),
                kind: GitRefKind::Head,
            });
            refs.push(parse_ref(target));
        } else {
            refs.push(parse_ref(decoration));
        }
    }
    refs
}

fn parse_ref(decoration: &str) -> GitRefLabel {
    if decoration == "HEAD" {
        GitRefLabel {
            name: decoration.to_string(),
            kind: GitRefKind::Head,
        }
    } else if let Some(name) = decoration.strip_prefix("refs/heads/") {
        GitRefLabel {
            name: name.to_string(),
            kind: GitRefKind::LocalBranch,
        }
    } else if let Some(name) = decoration.strip_prefix("refs/remotes/") {
        GitRefLabel {
            name: name.to_string(),
            kind: GitRefKind::RemoteBranch,
        }
    } else if let Some(name) = decoration.strip_prefix("tag: refs/tags/") {
        GitRefLabel {
            name: name.to_string(),
            kind: GitRefKind::Tag,
        }
    } else {
        GitRefLabel {
            name: decoration.to_string(),
            kind: GitRefKind::Other,
        }
    }
}

pub async fn load_repository(repo_path: &Path) -> Result<RepositorySnapshot> {
    let has_head = run_git_command(repo_path, &["rev-parse", "--verify", "HEAD"])
        .await
        .is_ok();
    let history = load_history(repo_path, 0, HISTORY_PAGE_SIZE, has_head);
    let staged = run_git_command(repo_path, &["diff", "--cached", "--name-status", "-z"]);
    let changes = run_git_command(repo_path, &["diff", "--name-status", "-z"]);
    let untracked = run_git_command(
        repo_path,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    );
    let (staged, changes, untracked, (commits, has_more_history)) =
        futures::try_join!(staged, changes, untracked, history)
            .with_context(|| format!("Unable to read Git state in {}", repo_path.display()))?;

    Ok(group_snapshot(
        parse_name_status_z(&staged),
        parse_name_status_z(&changes),
        parse_untracked_z(&untracked),
        commits,
        has_more_history,
        has_head,
    ))
}

pub async fn load_history(
    repo_path: &Path,
    skip: usize,
    page_size: usize,
    has_head: bool,
) -> Result<(Vec<CommitNode>, bool)> {
    if !has_head {
        return Ok((Vec::new(), false));
    }

    let limit = (page_size + 1).to_string();
    let skip = skip.to_string();
    let pretty = format!("--pretty=format:{HISTORY_FORMAT}");
    let output = run_git_command(
        repo_path,
        &[
            "log",
            "--date-order",
            "--decorate=full",
            "--no-color",
            "-n",
            &limit,
            "--skip",
            &skip,
            &pretty,
            "--all",
        ],
    )
    .await
    .with_context(|| format!("Unable to read Git history in {}", repo_path.display()))?;

    let mut commits = parse_history(&output);
    let has_more = commits.len() > page_size;
    commits.truncate(page_size);
    Ok((commits, has_more))
}

fn group_snapshot(
    staged: Vec<FileChange>,
    changes: Vec<FileChange>,
    untracked: Vec<FileChange>,
    commits: Vec<CommitNode>,
    has_more_history: bool,
    has_head: bool,
) -> RepositorySnapshot {
    let mut merge_changes = Vec::new();
    let mut seen_conflicts = HashSet::new();
    for change in staged.iter().chain(&changes) {
        if change.kind == GitChangeKind::Conflicted && seen_conflicts.insert(change.path.clone()) {
            merge_changes.push(change.clone());
        }
    }

    let mut staged_changes: Vec<_> = staged
        .into_iter()
        .filter(|change| change.kind != GitChangeKind::Conflicted)
        .collect();
    let mut changes: Vec<_> = changes
        .into_iter()
        .filter(|change| change.kind != GitChangeKind::Conflicted)
        .collect();
    let mut untracked_changes = untracked;
    sort_changes(&mut merge_changes);
    sort_changes(&mut staged_changes);
    sort_changes(&mut changes);
    sort_changes(&mut untracked_changes);

    RepositorySnapshot {
        merge_changes,
        staged_changes,
        changes,
        untracked_changes,
        commits,
        has_more_history,
        has_head,
    }
}

fn sort_changes(changes: &mut [FileChange]) {
    changes.sort_by(|left, right| left.path.cmp(&right.path));
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitMutation {
    StagePaths(Vec<String>),
    StageAll,
    UnstagePaths(Vec<String>),
    UnstageAll,
}

impl GitMutation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::StagePaths(_) => "stage file",
            Self::StageAll => "stage all changes",
            Self::UnstagePaths(_) => "unstage file",
            Self::UnstageAll => "unstage all changes",
        }
    }
}

pub async fn apply_mutation(
    repo_path: &Path,
    mutation: &GitMutation,
    has_head: bool,
) -> Result<()> {
    let args = mutation_args(mutation, has_head);
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    run_git_command(repo_path, &args).await?;
    Ok(())
}

fn mutation_args(mutation: &GitMutation, has_head: bool) -> Vec<String> {
    let mut args: Vec<String> = match mutation {
        GitMutation::StagePaths(_) => vec!["add", "--"],
        GitMutation::StageAll => vec!["add", "-A"],
        GitMutation::UnstagePaths(_) if has_head => vec!["reset", "-q", "HEAD", "--"],
        GitMutation::UnstageAll if has_head => vec!["reset", "-q", "HEAD", "--"],
        GitMutation::UnstagePaths(_) => {
            vec!["rm", "--cached", "-q", "--ignore-unmatch", "--"]
        }
        GitMutation::UnstageAll => vec!["rm", "--cached", "-q", "-r", "--", "."],
    }
    .into_iter()
    .map(ToString::to_string)
    .collect();
    match mutation {
        GitMutation::StagePaths(paths) | GitMutation::UnstagePaths(paths) => {
            args.extend(paths.iter().cloned());
        }
        GitMutation::StageAll | GitMutation::UnstageAll => {}
    }
    args
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
