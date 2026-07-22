use std::collections::HashMap;
#[cfg(feature = "local_fs")]
use std::path::Path;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Describes a local automation file that failed to parse or validate.
///
/// Carries enough context to surface a helpful error row / toast so the user
/// can locate and fix the broken TOML. Mirrors `TabConfigError`.
#[derive(Clone, Debug)]
pub struct LocalAutomationError {
    /// The file name shown to the user (e.g. `"morning_brief.toml"`).
    pub file_name: String,
    /// Full path used by the "Open config" action.
    pub file_path: PathBuf,
    /// The full, untruncated parse/validation error.
    pub error_message: String,
}

/// How a local automation executes when run.
///
/// Declared in TOML as a `[runner]` table with a `type` field:
///
/// ```toml
/// [runner]
/// type = "warp_agent"
/// prompt = "Summarize commits on main from the last 24h."
/// ```
///
/// or:
///
/// ```toml
/// [runner]
/// type = "shell"
/// command = "gh pr list --author @me"
/// ```
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalAutomationRunner {
    /// Run a local Warp agent conversation seeded with `prompt`.
    ///
    /// Run now opens a new local agent tab in the resolved working directory
    /// and starts the agent with the prompt under a CLI-like unattended
    /// execution profile (no interactive permission prompts).
    WarpAgent { prompt: String },
    /// Run a shell command in a new terminal tab at the resolved working
    /// directory. Third-party CLIs (Claude Code, Codex, etc.) are expressed
    /// as shell commands.
    Shell { command: String },
}

impl LocalAutomationRunner {
    /// Short human-readable runner label for list UIs.
    pub fn display_label(&self) -> &'static str {
        match self {
            LocalAutomationRunner::WarpAgent { .. } => "Warp agent",
            LocalAutomationRunner::Shell { .. } => "Shell",
        }
    }
}

/// A git worktree working directory, created (if needed) under the shared
/// Warp worktree root: `~/.warp/worktrees/<repo-name>/<name>`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalAutomationWorktree {
    /// Path to the git repository the worktree is created from (supports `~`).
    pub repo: String,
    /// Worktree (and new branch) name under the Warp worktree root.
    pub name: String,
    /// Optional branch/commit the new worktree branch is created from.
    /// Defaults to the repo's current `HEAD` when omitted.
    #[serde(default)]
    pub base_branch: Option<String>,
}

/// A local automation loaded from a `.toml` file in the user's `automations/`
/// data directory. One file defines exactly one automation.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAutomation {
    /// Display name shown in list UIs. May differ from the filename.
    pub name: String,
    /// Whether the automation is considered active for (future) scheduling.
    /// Run now still works on disabled automations, with a warning.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Cron expression or preset string. Stored for forward compatibility;
    /// Slice A does not fire automations on a schedule.
    pub schedule: String,
    /// Working directory for the run (supports `~`). Exactly one of `cwd` or
    /// `worktree` must be set.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Git worktree working directory. Exactly one of `cwd` or `worktree`
    /// must be set.
    #[serde(default)]
    pub worktree: Option<LocalAutomationWorktree>,
    /// How the automation executes.
    pub runner: LocalAutomationRunner,
    /// Optional run timeout. Stored in Slice A; enforced once scheduling
    /// lands.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// Optional environment variables for the run. Stored in Slice A; applied
    /// once the runner plumbing supports per-run env.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// The on-disk path this automation was loaded from.
    /// Populated during parsing; not serialized into or from the TOML.
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

fn default_enabled() -> bool {
    true
}

impl LocalAutomation {
    /// Parses and validates a local automation from TOML contents.
    pub fn parse(contents: &str) -> Result<Self, String> {
        let automation: LocalAutomation = toml::from_str(contents).map_err(|e| e.to_string())?;
        automation.validate()?;
        Ok(automation)
    }

    /// Validates cross-field constraints that serde cannot express.
    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("'name' must not be empty".to_string());
        }
        if self.schedule.trim().is_empty() {
            return Err("'schedule' must not be empty".to_string());
        }
        match (&self.cwd, &self.worktree) {
            (Some(_), Some(_)) => {
                return Err("exactly one of 'cwd' or '[worktree]' must be set, not both".to_string())
            }
            (None, None) => {
                return Err("exactly one of 'cwd' or '[worktree]' must be set".to_string())
            }
            (Some(cwd), None) => {
                if cwd.trim().is_empty() {
                    return Err("'cwd' must not be empty".to_string());
                }
            }
            (None, Some(worktree)) => {
                if worktree.repo.trim().is_empty() {
                    return Err("'worktree.repo' must not be empty".to_string());
                }
                if worktree.name.trim().is_empty() {
                    return Err("'worktree.name' must not be empty".to_string());
                }
            }
        }
        match &self.runner {
            LocalAutomationRunner::WarpAgent { prompt } => {
                if prompt.trim().is_empty() {
                    return Err(
                        "'runner.prompt' must not be empty for a warp_agent runner".to_string()
                    );
                }
            }
            LocalAutomationRunner::Shell { command } => {
                if command.trim().is_empty() {
                    return Err("'runner.command' must not be empty for a shell runner".to_string());
                }
            }
        }
        Ok(())
    }

    /// The path where a worktree working directory lives (or would live),
    /// under the shared Warp worktree root.
    pub fn worktree_path(worktree: &LocalAutomationWorktree) -> PathBuf {
        let repo = PathBuf::from(shellexpand::tilde(&worktree.repo).into_owned());
        crate::tab_configs::tab_config::generated_worktree_path(&repo, &worktree.name)
    }

    /// Resolves the working directory for a run, creating the git worktree if
    /// necessary.
    ///
    /// This may run `git` and block; call it from a background task. Errors
    /// are concrete, user-facing messages: a missing `cwd` fails rather than
    /// silently falling back to `$HOME`, and worktree setup failures include
    /// the underlying git error.
    #[cfg(feature = "local_fs")]
    pub fn resolve_working_directory(&self) -> Result<PathBuf, String> {
        match (&self.cwd, &self.worktree) {
            (Some(cwd), None) => {
                let expanded = PathBuf::from(shellexpand::tilde(cwd).into_owned());
                if !expanded.is_dir() {
                    return Err(format!(
                        "working directory {} does not exist",
                        expanded.display()
                    ));
                }
                Ok(expanded)
            }
            (None, Some(worktree)) => resolve_worktree(worktree),
            // Unreachable after validate(), but keep a concrete error rather
            // than panicking on hand-edited state.
            _ => Err("exactly one of 'cwd' or '[worktree]' must be set".to_string()),
        }
    }
}

/// Creates or reuses the automation's git worktree and returns its path.
///
/// An existing worktree directory is reused as-is (a dirty worktree still
/// runs; the user owns that risk in Slice A). Otherwise the worktree is
/// created with `git worktree add`, preferring a new branch named after the
/// worktree and falling back to checking out an existing branch of the same
/// name.
#[cfg(feature = "local_fs")]
fn resolve_worktree(worktree: &LocalAutomationWorktree) -> Result<PathBuf, String> {
    let repo = PathBuf::from(shellexpand::tilde(&worktree.repo).into_owned());
    if !repo.is_dir() {
        return Err(format!("worktree repo {} does not exist", repo.display()));
    }
    if !repo.join(".git").exists() {
        return Err(format!("{} is not a git repository", repo.display()));
    }

    let target = LocalAutomation::worktree_path(worktree);
    if target.is_dir() {
        return Ok(target);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create worktree root {}: {e}", parent.display()))?;
    }

    // Prefer creating a fresh branch named after the worktree.
    let mut create_branch = vec![
        "worktree".to_string(),
        "add".to_string(),
        target.display().to_string(),
        "-b".to_string(),
        worktree.name.clone(),
    ];
    if let Some(base_branch) = &worktree.base_branch {
        create_branch.push(base_branch.clone());
    }
    let create_error = match run_git(&repo, &create_branch) {
        Ok(()) => return Ok(target),
        Err(e) => e,
    };

    // The branch may already exist (e.g. the worktree directory was removed
    // manually); fall back to checking it out.
    let checkout_existing = vec![
        "worktree".to_string(),
        "add".to_string(),
        target.display().to_string(),
        worktree.name.clone(),
    ];
    match run_git(&repo, &checkout_existing) {
        Ok(()) => Ok(target),
        Err(_) => Err(format!("failed to create worktree: {create_error}")),
    }
}

#[cfg(feature = "local_fs")]
fn run_git(repo: &Path, args: &[String]) -> Result<(), String> {
    let output = command::blocking::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(test)]
#[path = "local_automation_tests.rs"]
mod tests;
