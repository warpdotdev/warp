//! Project progress context derived from `.ai/progress.md`.
//!
//! When a project contains `.ai/progress.md`, this module reads it to surface
//! the current and next tasks as structured AI context. Task status is
//! auto-derived from recent git commit messages so the context stays in sync
//! across every Warp window without manual maintenance.
//!
//! File layout expected under `<project>/.ai/`:
//!
//! ```text
//! .ai/
//! ├── progress.md          # task list: "[N] Task name  done|doing|todo"
//! └── ctx/
//!     ├── 00_goal.md       # project goal (optional)
//!     ├── 04_constraint.md # tech constraints (optional)
//!     ├── 05_api.md        # API spec (optional)
//!     └── 07_issue.md      # known issues log (optional)
//! ```
//!
//! A global fallback directory (`~/.ai/ctx/`) is consulted when a project-level
//! file is absent, matching the behaviour of the original aiflow shell tool.

use std::path::{Path, PathBuf};

use regex::Regex;

const PROGRESS_FILE: &str = ".ai/progress.md";
const CTX_DIR: &str = ".ai/ctx";
const GLOBAL_CTX_SUBDIR: &str = ".ai/ctx";

/// Status of a single task in `progress.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Done,
    Doing,
    Todo,
}

/// A parsed task entry from `progress.md`.
#[derive(Debug, Clone)]
pub struct ProgressTask {
    pub number: u32,
    pub name: String,
    pub status: TaskStatus,
}

/// All context assembled from a project's `.ai/` directory.
#[derive(Debug, Clone, Default)]
pub struct ProjectProgressContext {
    pub project_path: PathBuf,
    pub current_task: Option<ProgressTask>,
    pub next_task: Option<ProgressTask>,
    /// First 15 lines of `00_goal.md`, if present and non-empty.
    pub goal: Option<String>,
    /// First 10 lines of `05_api.md`, if present and non-empty.
    pub api_spec: Option<String>,
    /// First 10 lines of `04_constraint.md`, if present and non-empty.
    pub constraints: Option<String>,
    /// Last 3 non-blank lines of `07_issue.md`, if present and non-empty.
    pub recent_issues: Option<String>,
}

impl ProjectProgressContext {
    /// Maximum number of ancestor directories to walk when looking for
    /// `.ai/progress.md`. Bounds the I/O for pathological deep paths.
    const MAX_ANCESTOR_WALK: usize = 30;

    /// Loads progress context, starting at `start_dir` and walking up its
    /// ancestors until a directory containing `.ai/progress.md` is found.
    ///
    /// Returns `None` when no ancestor contains `.ai/progress.md`, or when
    /// the file exists but contains no parseable task entries — callers can
    /// cheaply skip the feature for projects that haven't opted in.
    pub fn load(start_dir: &Path) -> Option<Self> {
        let project_path = Self::find_project_root(start_dir)?;
        let progress_path = project_path.join(PROGRESS_FILE);

        let content = std::fs::read_to_string(&progress_path).ok()?;
        let mut tasks = Self::parse_progress(&content);

        // Best-effort: update task statuses from git history.
        if let Some(git_log) = Self::read_git_log_sync(&project_path) {
            Self::derive_status_from_git(&mut tasks, &git_log);
        }

        // No parseable tasks → skip injection entirely. The feature is opt-in,
        // so an empty / malformed file shouldn't surface placeholder context.
        if tasks.is_empty() {
            return None;
        }

        let current_task = tasks
            .iter()
            .find(|t| t.status == TaskStatus::Doing)
            .cloned();
        let next_task = tasks
            .iter()
            .find(|t| t.status == TaskStatus::Todo)
            .cloned();

        let ctx_dir = project_path.join(CTX_DIR);
        let global_ctx_dir = dirs::home_dir().map(|h| h.join(GLOBAL_CTX_SUBDIR));

        Some(ProjectProgressContext {
            project_path,
            current_task,
            next_task,
            goal: Self::read_ctx_head(&ctx_dir, global_ctx_dir.as_deref(), "00_goal.md", 15),
            api_spec: Self::read_ctx_head(&ctx_dir, global_ctx_dir.as_deref(), "05_api.md", 10),
            constraints: Self::read_ctx_head(
                &ctx_dir,
                global_ctx_dir.as_deref(),
                "04_constraint.md",
                10,
            ),
            recent_issues: Self::read_ctx_tail(
                &ctx_dir,
                global_ctx_dir.as_deref(),
                "07_issue.md",
                3,
            ),
        })
    }

    /// Walks ancestors of `start_dir` (including `start_dir` itself) and
    /// returns the first one containing `.ai/progress.md`.
    fn find_project_root(start_dir: &Path) -> Option<PathBuf> {
        start_dir
            .ancestors()
            .take(Self::MAX_ANCESTOR_WALK)
            .find(|dir| dir.join(PROGRESS_FILE).exists())
            .map(Path::to_path_buf)
    }

    /// Formats the context as a concise block (< 100 lines) suitable for
    /// injection into an AI session alongside other project context.
    pub fn to_formatted_string(&self) -> String {
        let mut lines: Vec<String> = Vec::new();

        // CURRENT is always emitted; it signals the active task even when absent.
        match &self.current_task {
            Some(t) => lines.push(format!("[CURRENT] [{}] {}", t.number, t.name)),
            None => lines.push("[CURRENT] (no task in progress)".to_string()),
        }

        if let Some(t) = &self.next_task {
            lines.push(format!("[NEXT]    [{}] {}", t.number, t.name));
        }

        if let Some(goal) = &self.goal {
            lines.push(String::new());
            lines.push("[GOAL]".to_string());
            lines.extend(goal.lines().take(15).map(str::to_owned));
        }

        if let Some(api) = &self.api_spec {
            lines.push(String::new());
            lines.push("[API]".to_string());
            lines.extend(api.lines().take(10).map(str::to_owned));
        }

        if let Some(c) = &self.constraints {
            lines.push(String::new());
            lines.push("[CONSTRAINTS]".to_string());
            lines.extend(c.lines().take(10).map(str::to_owned));
        }

        if let Some(issues) = &self.recent_issues {
            lines.push(String::new());
            lines.push("[RECENT ISSUES]".to_string());
            lines.push(issues.clone());
        }

        lines.join("\n")
    }

    // ── private helpers ───────────────────────────────────────────────────────

    /// Parses `progress.md` into a list of tasks.
    ///
    /// Each line must match `[N] Task name  done|doing|todo`.
    /// Comment lines, blank lines and headings are silently ignored so users
    /// can annotate their task list freely.
    fn parse_progress(content: &str) -> Vec<ProgressTask> {
        // Allow trailing whitespace and multiple spaces between fields.
        let re = Regex::new(r"^\[(\d+)\]\s+(.+?)\s+(done|doing|todo)\s*$")
            .expect("hardcoded regex is valid");

        content
            .lines()
            .filter_map(|line| {
                let caps = re.captures(line.trim())?;
                let number: u32 = caps[1].parse().ok()?;
                let name = caps[2].trim().to_owned();
                let status = match &caps[3] {
                    "done" => TaskStatus::Done,
                    "doing" => TaskStatus::Doing,
                    _ => TaskStatus::Todo,
                };
                Some(ProgressTask {
                    number,
                    name,
                    status,
                })
            })
            .collect()
    }

    /// Reads `git log --oneline -50` synchronously.
    ///
    /// Returns `None` on any failure (not a git repo, git not installed, etc.)
    /// so the caller falls back to the statuses already written in
    /// `progress.md` rather than crashing.
    fn read_git_log_sync(project_path: &Path) -> Option<String> {
        let output = command::blocking::Command::new("git")
            .args(["log", "--oneline", "-50"])
            .current_dir(project_path)
            .stdout(command::Stdio::piped())
            .stderr(command::Stdio::null())
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .ok()?;

        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            None
        }
    }

    /// Updates task statuses in-place using commit-message heuristics.
    ///
    /// Rules (matching the original aiflow shell logic):
    /// - A commit containing `[N]` or the task name (case-insensitive) marks
    ///   that task as `done`.
    /// - Tasks already marked `done` are left untouched.
    /// - The first remaining non-done task becomes `doing`.
    /// - All subsequent non-done tasks become `todo`.
    fn derive_status_from_git(tasks: &mut Vec<ProgressTask>, git_log: &str) {
        let log_lower = git_log.to_lowercase();
        let mut first_undone_found = false;

        for task in tasks.iter_mut() {
            if task.status == TaskStatus::Done {
                continue;
            }

            let bracket_marker = format!("[{}]", task.number);
            let name_lower = task.name.to_lowercase();

            let completed_in_git = git_log.contains(&bracket_marker)
                || log_lower.contains(&name_lower);

            if completed_in_git {
                task.status = TaskStatus::Done;
            } else if !first_undone_found {
                task.status = TaskStatus::Doing;
                first_undone_found = true;
            } else {
                task.status = TaskStatus::Todo;
            }
        }
    }

    /// Reads the first `max_lines` of a context file.
    ///
    /// Project-level (`ctx_dir`) takes priority over the global fallback
    /// (`global_ctx_dir`). Returns `None` when the file is absent or contains
    /// only comments / blank lines.
    fn read_ctx_head(
        ctx_dir: &Path,
        global_ctx_dir: Option<&Path>,
        name: &str,
        max_lines: usize,
    ) -> Option<String> {
        let path = Self::resolve_ctx_file(ctx_dir, global_ctx_dir, name)?;
        let content = std::fs::read_to_string(&path).ok()?;
        if Self::has_meaningful_content(&content) {
            Some(
                content
                    .lines()
                    .take(max_lines)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        } else {
            None
        }
    }

    /// Reads the last `n` meaningful lines of a context file.
    fn read_ctx_tail(
        ctx_dir: &Path,
        global_ctx_dir: Option<&Path>,
        name: &str,
        n: usize,
    ) -> Option<String> {
        let path = Self::resolve_ctx_file(ctx_dir, global_ctx_dir, name)?;
        let content = std::fs::read_to_string(&path).ok()?;
        let meaningful: Vec<&str> = content
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#') && !t.starts_with("<!--")
            })
            .collect();

        if meaningful.is_empty() {
            return None;
        }

        let tail: Vec<&str> = meaningful
            .iter()
            .rev()
            .take(n)
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        Some(tail.join("\n"))
    }

    fn resolve_ctx_file(
        ctx_dir: &Path,
        global_ctx_dir: Option<&Path>,
        name: &str,
    ) -> Option<PathBuf> {
        let local = ctx_dir.join(name);
        if local.exists() {
            return Some(local);
        }
        if let Some(global) = global_ctx_dir {
            let fallback = global.join(name);
            if fallback.exists() {
                return Some(fallback);
            }
        }
        None
    }

    fn has_meaningful_content(content: &str) -> bool {
        content.lines().any(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#') && !t.starts_with("<!--")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_progress_basic() {
        let content = "
[1] Login UI         done
[2] API integration  doing
[3] Error handling   todo
[4] Unit tests       todo
";
        let tasks = ProjectProgressContext::parse_progress(content);
        assert_eq!(tasks.len(), 4);
        assert_eq!(tasks[0].status, TaskStatus::Done);
        assert_eq!(tasks[1].status, TaskStatus::Doing);
        assert_eq!(tasks[1].name, "API integration");
        assert_eq!(tasks[2].status, TaskStatus::Todo);
    }

    #[test]
    fn parse_progress_ignores_comments_and_blanks() {
        let content = "
# My project tasks
[1] Setup  done

# In progress
[2] Build  doing
";
        let tasks = ProjectProgressContext::parse_progress(content);
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn derive_status_marks_committed_tasks_done() {
        let mut tasks = vec![
            ProgressTask {
                number: 1,
                name: "Setup".to_owned(),
                status: TaskStatus::Todo,
            },
            ProgressTask {
                number: 2,
                name: "API integration".to_owned(),
                status: TaskStatus::Todo,
            },
            ProgressTask {
                number: 3,
                name: "Tests".to_owned(),
                status: TaskStatus::Todo,
            },
        ];
        let git_log = "abc1234 [1] Setup complete\ndef5678 some unrelated commit";
        ProjectProgressContext::derive_status_from_git(&mut tasks, git_log);

        assert_eq!(tasks[0].status, TaskStatus::Done);
        assert_eq!(tasks[1].status, TaskStatus::Doing);
        assert_eq!(tasks[2].status, TaskStatus::Todo);
    }

    #[test]
    fn load_walks_ancestors_to_find_progress_file() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".ai")).unwrap();
        std::fs::write(
            root.join(".ai/progress.md"),
            "[1] Setup  doing\n[2] Build  todo\n",
        )
        .unwrap();

        let nested = root.join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();

        let ctx = ProjectProgressContext::load(&nested).expect("should resolve from subdir");
        // The resolved root must be an ancestor of the starting directory and
        // must itself contain `.ai/progress.md`.
        assert!(
            nested.starts_with(&ctx.project_path),
            "resolved project_path must be an ancestor of the start dir"
        );
        assert!(ctx.project_path.join(".ai/progress.md").exists());
        assert!(ctx.current_task.is_some());
    }

    #[test]
    fn load_returns_none_when_no_progress_file_in_any_ancestor() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let nested = tmp.path().join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(ProjectProgressContext::load(&nested).is_none());
    }

    #[test]
    fn load_returns_none_when_progress_file_has_no_parseable_tasks() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".ai")).unwrap();
        // Only comments / blank lines / malformed lines — no valid task entries.
        std::fs::write(
            root.join(".ai/progress.md"),
            "# my project\n\nnot a task line\n[abc] bad number  doing\n",
        )
        .unwrap();

        assert!(
            ProjectProgressContext::load(root).is_none(),
            "empty / unparseable progress.md must not inject placeholder context"
        );
    }

    #[test]
    fn to_formatted_string_contains_current_and_next() {
        let ctx = ProjectProgressContext {
            project_path: PathBuf::from("/tmp/proj"),
            current_task: Some(ProgressTask {
                number: 2,
                name: "API integration".to_owned(),
                status: TaskStatus::Doing,
            }),
            next_task: Some(ProgressTask {
                number: 3,
                name: "Error handling".to_owned(),
                status: TaskStatus::Todo,
            }),
            ..Default::default()
        };
        let s = ctx.to_formatted_string();
        assert!(s.contains("[CURRENT]"));
        assert!(s.contains("API integration"));
        assert!(s.contains("[NEXT]"));
        assert!(s.contains("Error handling"));
    }
}
