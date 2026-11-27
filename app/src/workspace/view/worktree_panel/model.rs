//! Data model for the worktree panel: repositories, their git worktrees, and the coding agent
//! activity of the terminals running inside each worktree.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A git worktree checked out from a [`Repository`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    /// Short branch name, or `None` when `HEAD` is detached.
    pub branch: Option<String>,
    /// Whether this is the repository's main working tree rather than a linked worktree.
    pub is_main: bool,
}

impl Worktree {
    pub fn display_name(&self) -> String {
        match &self.branch {
            Some(branch) => branch.clone(),
            None => self
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.path.display().to_string()),
        }
    }
}

/// A git repository, identified by the path of its main working tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    pub root: PathBuf,
    pub name: String,
    /// Worktrees with the main working tree first.
    pub worktrees: Vec<Worktree>,
}

impl Repository {
    /// Builds a repository from parsed worktrees, returning `None` if there is no main worktree.
    pub fn from_worktrees(worktrees: Vec<Worktree>) -> Option<Self> {
        let root = worktrees.iter().find(|w| w.is_main)?.path.clone();
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        Some(Self {
            root,
            name,
            worktrees,
        })
    }
}

/// Parses the output of `git worktree list --porcelain`. Git always lists the main working tree
/// first; bare repositories have no working tree and are skipped.
pub fn parse_worktree_porcelain(output: &str) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    for (index, record) in output
        .split("\n\n")
        .filter(|record| !record.trim().is_empty())
        .enumerate()
    {
        let mut path = None;
        let mut branch = None;
        let mut is_bare = false;
        for line in record.lines() {
            if let Some(value) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(value));
            } else if let Some(value) = line.strip_prefix("branch ") {
                branch = Some(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_owned(),
                );
            } else if line == "bare" {
                is_bare = true;
            }
        }
        if let Some(path) = path
            && !is_bare
        {
            worktrees.push(Worktree {
                path,
                branch,
                is_main: index == 0,
            });
        }
    }
    worktrees
}

/// How busy a coding agent is. Variants are ordered from least to most in need of attention so
/// the most urgent state wins when several agents share a worktree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AgentState {
    Idle,
    /// An agent CLI is running but does not report a detailed status.
    Running,
    Working,
    NeedsInput,
}

impl AgentState {
    pub fn label(self) -> &'static str {
        match self {
            AgentState::Idle => "Idle",
            AgentState::Running => "Active",
            AgentState::Working => "Working",
            AgentState::NeedsInput => "Needs input",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentActivity {
    pub name: String,
    pub state: AgentState,
}

/// A terminal session's location and agent activity, as reported by the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalActivity {
    pub tab_index: usize,
    pub cwd: PathBuf,
    pub agent: Option<AgentActivity>,
}

/// Activity rolled up across every terminal located in one worktree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorktreeActivity {
    pub agents: Vec<AgentActivity>,
    pub tab_indices: Vec<usize>,
}

impl WorktreeActivity {
    pub fn state(&self) -> Option<AgentState> {
        self.agents.iter().map(|agent| agent.state).max()
    }

    /// e.g. "Claude Code · Working" or "2 agents · Needs input".
    pub fn summary(&self) -> Option<String> {
        let state = self.state()?;
        let who = match self.agents.as_slice() {
            [agent] => agent.name.clone(),
            agents => format!("{} agents", agents.len()),
        };
        Some(format!("{who} · {}", state.label()))
    }
}

/// Returns the most specific worktree containing `cwd`. Longest-prefix matching matters when
/// worktrees are nested inside the main working tree (e.g. `<repo>/.worktrees/<branch>`).
pub fn worktree_containing<'a>(cwd: &Path, repositories: &'a [Repository]) -> Option<&'a Path> {
    repositories
        .iter()
        .flat_map(|repository| &repository.worktrees)
        .filter(|worktree| cwd.starts_with(&worktree.path))
        .max_by_key(|worktree| worktree.path.components().count())
        .map(|worktree| worktree.path.as_path())
}

pub fn aggregate_activity(
    terminals: &[TerminalActivity],
    repositories: &[Repository],
) -> HashMap<PathBuf, WorktreeActivity> {
    let mut activity: HashMap<PathBuf, WorktreeActivity> = HashMap::new();
    for terminal in terminals {
        let Some(worktree) = worktree_containing(&terminal.cwd, repositories) else {
            continue;
        };
        let entry = activity.entry(worktree.to_path_buf()).or_default();
        if !entry.tab_indices.contains(&terminal.tab_index) {
            entry.tab_indices.push(terminal.tab_index);
        }
        if let Some(agent) = &terminal.agent {
            entry.agents.push(agent.clone());
        }
    }
    activity
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
