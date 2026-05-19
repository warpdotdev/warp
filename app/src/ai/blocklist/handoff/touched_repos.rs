//! Touched-workspace derivation for local-to-cloud handoff (REMOTE-1486).
//!
//! Given an [`AIConversation`] (or the flat list of paths extracted from one) and
//! the user's currently-known cloud agent environments, this module produces:
//!
//! 1. The flat set of filesystem paths an agent run has touched, walked off the
//!    conversation's action history and the per-exchange `working_directory`
//!    (see [`extract_paths_from_conversation`]).
//! 2. A [`TouchedWorkspace`] enumerating the distinct git repos and orphan files the
//!    local agent has touched. Each repo carries a parsed `repo_id` (`<owner>/<repo>`)
//!    derived from its `origin` remote URL, fetched via an async `git` invocation so
//!    derivation never blocks the UI thread.
//! 3. A repo-aware default environment selection that layers on top of the existing
//!    cloud-agent setup recency-sort.
//!
//! Path extraction is sync and pure (no I/O), and the workspace derivation is async
//! (one `git remote get-url origin` per unique repo). Callers run them in sequence
//! off the main thread; see `app/src/workspace/view.rs::start_local_to_cloud_handoff`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use command::r#async::Command;
use command::Stdio;
use futures::future::join_all;
use tokio::fs as tokio_fs;
use warpui::r#async::FutureExt as _;

use crate::ai::agent::conversation::AIConversation;
use crate::ai::agent::{AIAgentAction, AIAgentActionType, AIAgentOutputMessageType};
use crate::ai::blocklist::agent_view::agent_input_footer::sort_environments_by_recency;
use crate::ai::cloud_environments::{CloudAmbientAgentEnvironment, GithubRepo};
use crate::server::ids::SyncId;

/// Cap on how many of the conversation's action results we scan for paths,
/// counted from most-recent backwards. Conversations with more than this many
/// tool calls only contribute paths from their most recent
/// [`MAX_TOOL_CALLS_TO_SCAN`].
pub(crate) const MAX_TOOL_CALLS_TO_SCAN: usize = 500;

/// Soft cap on each git invocation we dispatch. Mirrors the cap used by the cloud-side
/// snapshot pipeline so individual filesystem hiccups don't stall the modal indefinitely.
const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// The collection of git repos and orphan files the local agent has touched in the
/// active conversation. Drives both the snapshot upload plan and the modal's env-
/// overlap status row.
#[derive(Clone, Debug, Default)]
pub(crate) struct TouchedWorkspace {
    pub repos: Vec<TouchedRepo>,
    /// Files touched outside any `.git` directory.
    /// They're captured as raw file contents in the snapshot manifest.
    pub orphan_files: Vec<PathBuf>,
}

/// A single git repo touched by the local agent.
#[derive(Clone, Debug)]
pub(crate) struct TouchedRepo {
    /// Absolute path to the working tree root (the directory containing `.git`).
    pub git_root: PathBuf,
    /// `<owner>/<repo>` parsed from the `origin` remote URL, when discoverable.
    /// Drives env-overlap matching against `CloudAmbientAgentEnvironment.github_repos`
    /// and the modal's per-repo status row label.
    pub repo_id: Option<GithubRepo>,
}

/// Derive the `TouchedWorkspace` from a flat list of absolute paths.
///
/// Walks each path up to the nearest `.git` directory; paths whose walk-up doesn't
/// find one go into `orphan_files`. For each unique git root, runs
/// `git remote get-url origin` to parse out the `<owner>/<repo>` for env-overlap
/// matching. Errors on the git call are non-fatal — `repo_id` stays `None`.
///
/// `paths` must already be absolute and must come from
/// [`extract_paths_from_conversation`], which only emits paths the agent
/// actually wrote to (plus per-exchange cwds for repo discovery). That gate
/// is what makes the orphan-file branch safe — we never stage a read-only
/// path like `~/.ssh/id_rsa` for upload.
pub(crate) async fn derive_touched_workspace(paths: Vec<PathBuf>) -> TouchedWorkspace {
    if paths.is_empty() {
        return TouchedWorkspace::default();
    }

    let mut git_roots: Vec<PathBuf> = Vec::new();
    let mut orphan_files: Vec<PathBuf> = Vec::new();
    let mut seen_roots: HashSet<PathBuf> = HashSet::new();

    for path in paths {
        match find_git_root(&path).await {
            Some(root) => {
                if seen_roots.insert(root.clone()) {
                    git_roots.push(root);
                }
            }
            None => {
                if tokio_fs::metadata(&path).await.is_ok_and(|m| m.is_file()) {
                    orphan_files.push(path);
                }
            }
        }
    }

    let metadata_futures = git_roots.into_iter().map(|git_root| async move {
        let repo_id = git_origin_url(&git_root)
            .await
            .as_deref()
            .and_then(parse_github_repo);
        TouchedRepo { git_root, repo_id }
    });
    let repos: Vec<TouchedRepo> = join_all(metadata_futures).await;

    TouchedWorkspace {
        repos,
        orphan_files,
    }
}

/// Walk `path` up to find the nearest enclosing `.git` directory and return its parent
/// (the working-tree root). Returns `None` if no `.git` is found.
async fn find_git_root(path: &Path) -> Option<PathBuf> {
    let mut cursor: Option<&Path> = if tokio_fs::metadata(path).await.is_ok_and(|m| m.is_dir()) {
        Some(path)
    } else {
        path.parent()
    };
    while let Some(dir) = cursor {
        let candidate = dir.join(".git");
        if tokio_fs::try_exists(&candidate).await.unwrap_or(false) {
            return Some(dir.to_path_buf());
        }
        cursor = dir.parent();
    }
    None
}

/// Run `git remote get-url origin` in `git_root` with a bounded timeout, returning the
/// trimmed remote URL or `None` if the invocation fails, times out, exits non-zero, or
/// yields empty/non-UTF-8 output. [`GIT_COMMAND_TIMEOUT`] caps the call so a stalled git
/// process can't pin the loading state forever.
async fn git_origin_url(git_root: &Path) -> Option<String> {
    let mut command = Command::new("git");
    command
        .args(["remote", "get-url", "origin"])
        .current_dir(git_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let Ok(Ok(output)) = command.output().with_timeout(GIT_COMMAND_TIMEOUT).await else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Parse a GitHub remote URL of either the SSH (`git@github.com:owner/repo.git`) or
/// HTTPS (`https://github.com/owner/repo[.git]`) flavor into a [`GithubRepo`].
/// Returns `None` for non-GitHub remotes (we only support env-overlap for GitHub today,
/// matching the env-creation flow).
fn parse_github_repo(remote_url: &str) -> Option<GithubRepo> {
    let trimmed = remote_url.trim();
    let path_part = if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("ssh://git@github.com/") {
        rest
    } else {
        return None;
    };

    let path_part = path_part.strip_suffix(".git").unwrap_or(path_part);
    let mut segments = path_part.splitn(2, '/');
    let owner = segments.next()?.to_string();
    let repo = segments.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(GithubRepo::new(owner, repo))
}

/// Resolve a single directory path to its enclosing git repo and parsed GitHub
/// remote, if any.
pub(crate) async fn resolve_repo_for_path(path: &Path) -> Option<TouchedRepo> {
    let git_root = find_git_root(path).await?;
    let repo_id = git_origin_url(&git_root)
        .await
        .as_deref()
        .and_then(parse_github_repo);
    Some(TouchedRepo { git_root, repo_id })
}

/// Pick the env that has the most overlap with the touched repos, breaking ties by
/// recency. Returns `None` when no env contains any of the touched repos (or when
/// `envs` is empty / the workspace touched no GitHub-mapped repos).
///
/// This is the "strict" overlap-aware pick used by the handoff pane bootstrap,
/// which calls it unconditionally and applies the result on top of whatever the
/// `EnvironmentSelector`'s `ensure_default_selection` had already picked. When
/// this returns `None`, callers leave the existing selection alone.
pub(crate) fn pick_handoff_overlap_env(
    workspace: &TouchedWorkspace,
    mut envs: Vec<CloudAmbientAgentEnvironment>,
) -> Option<SyncId> {
    if envs.is_empty() {
        return None;
    }

    let touched_repo_ids: Vec<&GithubRepo> = workspace
        .repos
        .iter()
        .filter_map(|r| r.repo_id.as_ref())
        .collect();
    if touched_repo_ids.is_empty() {
        return None;
    }

    // Sort most-recent-first so that ties on overlap count resolve to the most-
    // recently-used env. We then iterate and keep the first-best score.
    sort_environments_by_recency(&mut envs);
    let mut best: Option<(&CloudAmbientAgentEnvironment, usize)> = None;
    for env in &envs {
        let env_repos = &env.model().string_model.github_repos;
        let score = touched_repo_ids
            .iter()
            .filter(|id| env_repos.iter().any(|r| &r == *id))
            .count();
        if score == 0 {
            continue;
        }
        match best {
            None => best = Some((env, score)),
            Some((_, current)) if score > current => best = Some((env, score)),
            _ => {}
        }
    }
    best.map(|(env, _)| env.id)
}

// --- Path extraction from `AIConversation` ---
//
// Walks an [`AIConversation`] and collects the filesystem paths the local agent
// actually wrote to, plus the per-exchange `working_directory`. The output
// feeds [`derive_touched_workspace`], which groups paths by enclosing `.git`
// repo and produces the [`TouchedWorkspace`] the orchestrator uploads from.
//
// Read-only actions (`ReadFiles`, `Grep`, `FileGlob*`, `SearchCodebase`,
// `InsertCodeReviewComments`) are intentionally NOT walked. The handoff
// snapshot uploads orphan-file contents verbatim, so including a read-only
// reference like `~/.ssh/id_rsa` would leak unrelated local files into the
// cloud agent. Limiting the walk to writes (`RequestFileEdits`,
// `UploadArtifact`) keeps the snapshot to files the user knowingly let the
// agent author. Repos the agent only browsed are still discoverable through
// the per-exchange cwd, which is captured below.
//
// `Path::is_absolute()` paths pass through unchanged; relative paths are
// resolved against the exchange's `working_directory` (and dropped when there
// is no cwd to resolve against). Empty entries are dropped.
//
// Cost is bounded by walking only the [`MAX_TOOL_CALLS_TO_SCAN`] most recent
// action results across all exchanges. Older actions are skipped under the
// assumption that the workspace state the user wants to hand off is dominated
// by recent work; this keeps very long conversations from paying an unbounded
// per-handoff scan cost.

/// Collect every filesystem path the agent wrote to in any of the conversation's
/// write actions (plus the cwd of every exchange that ran shell commands),
/// capped to the most recent [`MAX_TOOL_CALLS_TO_SCAN`] action results.
///
/// The returned vec is deduplicated and may contain both absolute and
/// resolved-against-`working_directory` paths. Per-path filesystem checks
/// (does the path exist? does it have a `.git` ancestor?) happen later in
/// [`derive_touched_workspace`].
pub(crate) fn extract_paths_from_conversation(conversation: &AIConversation) -> Vec<PathBuf> {
    // Walk exchanges newest-first so we can stop once we've consumed the cap.
    // Within each exchange we count every `Action` message against the budget
    // and bail early if we hit it mid-exchange.
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut tool_calls_remaining = MAX_TOOL_CALLS_TO_SCAN;

    for exchange in conversation.all_exchanges().into_iter().rev() {
        if tool_calls_remaining == 0 {
            break;
        }
        let cwd = exchange.working_directory.as_deref();

        // Track the per-exchange cwd unconditionally (it doesn't count as a tool
        // call). Covers `RunShellCommand` cwds without walking action results.
        if let Some(cwd) = cwd {
            let cwd_path = PathBuf::from(cwd);
            if cwd_path.is_absolute() && seen.insert(cwd_path.clone()) {
                paths.push(cwd_path);
            }
        }

        let Some(output) = exchange.output_status.output() else {
            continue;
        };
        let output = output.get();
        // Walk messages newest-first within the exchange too, so a single long
        // exchange can't burn the budget on its oldest tool calls before
        // reaching its most recent edits.
        for message in output.messages.iter().rev() {
            let AIAgentOutputMessageType::Action(action) = &message.message else {
                continue;
            };
            if tool_calls_remaining == 0 {
                break;
            }
            tool_calls_remaining -= 1;
            extract_action_paths(action, cwd, &mut paths, &mut seen);
        }
    }

    paths
}

fn extract_action_paths(
    action: &AIAgentAction,
    cwd: Option<&str>,
    paths: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) {
    match &action.action {
        // Write actions: the agent authored or replaced these files. Safe to
        // stage as orphan-file content if they fall outside any git repo.
        AIAgentActionType::RequestFileEdits { file_edits, .. } => {
            for edit in file_edits {
                push_resolved(edit.file(), cwd, paths, seen);
            }
        }
        AIAgentActionType::UploadArtifact(req) => {
            push_resolved(Some(req.file_path.as_str()), cwd, paths, seen);
        }
        // Read / search actions are intentionally NOT walked. See module-level
        // comment: including read-only references would let `ReadFiles` on
        // something like `~/.ssh/id_rsa` leak into the snapshot upload.
        AIAgentActionType::ReadFiles(_)
        | AIAgentActionType::Grep { .. }
        | AIAgentActionType::FileGlob { .. }
        | AIAgentActionType::FileGlobV2 { .. }
        | AIAgentActionType::SearchCodebase(_)
        | AIAgentActionType::InsertCodeReviewComments { .. }
        | AIAgentActionType::RequestCommandOutput { .. }
        | AIAgentActionType::WriteToLongRunningShellCommand { .. }
        | AIAgentActionType::ReadShellCommandOutput { .. }
        | AIAgentActionType::ReadMCPResource { .. }
        | AIAgentActionType::CallMCPTool { .. }
        | AIAgentActionType::SuggestNewConversation { .. }
        | AIAgentActionType::SuggestPrompt(_)
        | AIAgentActionType::InitProject
        | AIAgentActionType::OpenCodeReview
        | AIAgentActionType::ReadDocuments(_)
        | AIAgentActionType::EditDocuments(_)
        | AIAgentActionType::CreateDocuments(_)
        | AIAgentActionType::UseComputer(_)
        | AIAgentActionType::RequestComputerUse(_)
        | AIAgentActionType::ReadSkill(_)
        | AIAgentActionType::FetchConversation { .. }
        | AIAgentActionType::StartAgent { .. }
        | AIAgentActionType::SendMessageToAgent { .. }
        | AIAgentActionType::TransferShellCommandControlToUser { .. }
        | AIAgentActionType::AskUserQuestion { .. }
        | AIAgentActionType::RunAgents(_) => {}
    }
}

/// Push `raw` into `paths` after resolving it against `cwd` if necessary.
/// Empty / `None` entries are ignored.
fn push_resolved(
    raw: Option<&str>,
    cwd: Option<&str>,
    paths: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) {
    let Some(raw) = raw else { return };
    let raw = raw.trim();
    if raw.is_empty() {
        return;
    }
    let candidate = Path::new(raw);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else if let Some(cwd) = cwd {
        Path::new(cwd).join(candidate)
    } else {
        // No cwd context, no absolute path — we have nothing actionable.
        return;
    };
    if seen.insert(resolved.clone()) {
        paths.push(resolved);
    }
}

#[cfg(test)]
#[path = "touched_repos_tests.rs"]
mod tests;
