use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use warp_util::git::run_git_command;

pub const HISTORY_PAGE_SIZE: usize = 50;

const FIELD_SEPARATOR: char = '\u{1f}';
const RECORD_SEPARATOR: char = '\u{1e}';
const HISTORY_FORMAT: &str = "%x1f%H%x1f%P%x1f%an%x1f%at%x1f%s%x1f%D%x1f%b%x1e";

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
pub struct CommitStats {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitNode {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: i64,
    pub subject: String,
    pub body: String,
    pub refs: Vec<GitRefLabel>,
    pub stats: Option<CommitStats>,
}

impl CommitNode {
    pub fn short_hash(&self) -> &str {
        self.hash.get(..7).unwrap_or(&self.hash)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepositorySnapshot {
    pub commits: Vec<CommitNode>,
    pub has_more_history: bool,
    pub has_head: bool,
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
            let body = fields.collect::<Vec<_>>().join("\u{1f}");
            let body = body.trim_end().to_string();

            (!hash.is_empty()).then_some(CommitNode {
                hash,
                parents,
                author,
                timestamp,
                subject,
                body,
                refs,
                stats: None,
            })
        })
        .collect()
}

pub fn parse_shortstat_log(output: &str) -> HashMap<String, CommitStats> {
    output
        .split(RECORD_SEPARATOR)
        .filter_map(|record| {
            let mut lines = record
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty());
            let hash = lines.next()?.to_string();
            let stat_line = lines.find(|line| line.contains(" changed"))?;

            let mut files_changed = None;
            let mut insertions = 0;
            let mut deletions = 0;
            for part in stat_line.split(',').map(str::trim) {
                let Some(count) = part
                    .split_whitespace()
                    .next()
                    .and_then(|count| count.parse::<usize>().ok())
                else {
                    continue;
                };
                if part.contains("file changed") || part.contains("files changed") {
                    files_changed = Some(count);
                } else if part.contains("insertion") {
                    insertions = count;
                } else if part.contains("deletion") {
                    deletions = count;
                }
            }

            files_changed.map(|files_changed| {
                (
                    hash,
                    CommitStats {
                        files_changed,
                        insertions,
                        deletions,
                    },
                )
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
    let (commits, has_more_history) =
        load_history(repo_path, 0, HISTORY_PAGE_SIZE, has_head).await?;

    Ok(RepositorySnapshot {
        commits,
        has_more_history,
        has_head,
    })
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
    let history_args = [
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
    ];
    let shortstat_args = [
        "log",
        "--date-order",
        "--no-color",
        "--shortstat",
        "--format=%x1e%H",
        "-n",
        &limit,
        "--skip",
        &skip,
        "--all",
    ];
    let history = run_git_command(repo_path, &history_args);
    let shortstat = run_git_command(repo_path, &shortstat_args);
    let (output, shortstat_output) = futures::join!(history, shortstat);
    let output =
        output.with_context(|| format!("Unable to read Git history in {}", repo_path.display()))?;

    let mut commits = parse_history(&output);
    if let Ok(shortstat_output) = shortstat_output {
        let stats = parse_shortstat_log(&shortstat_output);
        for commit in &mut commits {
            commit.stats = stats.get(&commit.hash).cloned();
        }
    }
    let has_more = commits.len() > page_size;
    commits.truncate(page_size);
    Ok((commits, has_more))
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
