# Tech Spec: Project progress context for AI sessions (`.ai/progress.md`)

**Issue:** warpdotdev/warp#10342

## Context

### The problem in code

Every AI query assembles context in
`app/src/ai/blocklist/context_model.rs:pending_context()` (line ~419).
Currently this function injects:

- `AIAgentContext::Directory` — working directory
- `AIAgentContext::Git` — current branch / HEAD
- `AIAgentContext::ProjectRules` — contents of `WARP.md` / `AGENTS.md` (via
  `ProjectContextModel`)

There is no mechanism to carry task-level progress state between sessions.
Each window starts from zero context about *what the developer is building*.

### Existing infrastructure to reuse

| Component | Location | Reuse |
|-----------|----------|-------|
| `ProjectContextModel` | `crates/ai/src/project_context/model.rs` | Pattern reference for file-watching + singleton |
| `AIAgentContext::ProjectRules` | `app/src/ai/agent/mod.rs:1978` | Transport for the new context (no new enum variant needed) |
| `convert_context()` | `app/src/ai/agent/api/convert_to.rs:723` | Already serialises `ProjectRules` to the server API |
| `run_git_command_with_env` / `list_local_branches_sync` | `app/src/util/git.rs` | Pattern for synchronous git subprocess calls |
| `command::blocking::Command` | `crates/command/` | Sandboxed subprocess wrapper (already in `app` crate) |

### Why no new `AIAgentContext` variant

Adding a variant would require a matching proto change in `warp_multi_agent_api`
(a private repo) and changes to the server. Re-using `ProjectRules` as the
transport keeps the change entirely client-side and reviewable in one PR.

## Proposed changes

### New file: `app/src/ai/project_progress.rs`

Self-contained module with no warpui/model dependencies.

**Key types:**

```rust
pub enum TaskStatus { Done, Doing, Todo }

pub struct ProgressTask { pub number: u32, pub name: String, pub status: TaskStatus }

pub struct ProjectProgressContext {
    pub project_path: PathBuf,
    pub current_task: Option<ProgressTask>,
    pub next_task:    Option<ProgressTask>,
    pub goal:         Option<String>,   // 00_goal.md, 15 lines
    pub api_spec:     Option<String>,   // 05_api.md,  10 lines
    pub constraints:  Option<String>,   // 04_constraint.md, 10 lines
    pub recent_issues: Option<String>,  // 07_issue.md, last 3 lines
}
```

**`ProjectProgressContext::load(project_path: &Path) -> Option<Self>`**

1. Return `None` if `<project>/.ai/progress.md` does not exist.
2. Parse `progress.md` with regex `^\[(\d+)\]\s+(.+?)\s+(done|doing|todo)\s*$`.
3. Call `read_git_log_sync` → `derive_status_from_git` (best-effort, silent
   on failure).
4. Read optional ctx files from `<project>/.ai/ctx/`, falling back to
   `~/.ai/ctx/` via `dirs::home_dir()`.
5. Return assembled struct.

**`to_formatted_string(&self) -> String`**

Renders the struct into the structured block injected into the AI session.
Always emits `[CURRENT]`; other sections are conditional on presence.
Total output is capped at ~100 lines by the per-section line limits.

**`derive_status_from_git`**

For each non-done task, searches `git log --oneline -50` for `[N]` or the
task name (case-insensitive). First unmatched task → `Doing`. Rest → `Todo`.
Matches the shell logic in the original aiflow implementation.

### Modified: `app/src/ai/mod.rs`

Add one line:

```rust
pub(crate) mod project_progress;
```

### Modified: `app/src/ai/blocklist/context_model.rs`

In `pending_context()`, after the `ProjectRules` injection block (line ~492):

```rust
// Inject project progress context when `.ai/progress.md` is present.
if let Some(progress) = canonical_pwd.as_deref().and_then(ProjectProgressContext::load) {
    let formatted = progress.to_formatted_string();
    let line_count = formatted.lines().count();
    context.push(AIAgentContext::ProjectRules {
        root_path: progress.project_path.to_string_lossy().into(),
        active_rules: vec![FileContext {
            file_name: ".ai/progress.md".to_owned(),
            content: AnyFileContent::StringContent(formatted),
            line_range: None,
            last_modified: None,
            line_count,
        }],
        additional_rule_paths: vec![],
    });
}
```

Also refactors the `pwd`-canonicalisation into a shared `canonical_pwd`
binding to avoid duplicating the `PathBuf::from_str` + `canonicalize` chain.

### No changes needed

- `convert_to.rs` — `AIAgentContext::ProjectRules` is already handled.
- `convert_conversation.rs` — same.
- `Cargo.toml` files — all dependencies (`regex`, `dirs`, `command`) are
  already in the `ai` / `app` workspace.

## End-to-end flow

```
User opens Warp AI in window with pwd = /home/user/myproject
  │
  ├── pending_context() called
  │     ├── canonical_pwd = /home/user/myproject
  │     ├── ProjectContextModel::find_applicable_rules → WARP.md rules
  │     └── ProjectProgressContext::load(/home/user/myproject)
  │           ├── reads .ai/progress.md
  │           ├── git log --oneline -50 (subprocess, sync)
  │           ├── derives task statuses
  │           ├── reads .ai/ctx/00_goal.md (optional)
  │           └── returns ProjectProgressContext { current: [2] Payment API, … }
  │
  └── context pushed to server as AIAgentContext::ProjectRules {
        file_name: ".ai/progress.md",
        content: "[CURRENT] [2] Payment API\n[NEXT] [3] Email …"
      }
```

## Testing and validation

### Unit tests (in `app/src/ai/project_progress.rs`)

Map to product spec invariants:

| Test | Invariant |
|------|-----------|
| `parse_progress_basic` | Tasks with `done`/`doing`/`todo` parsed correctly |
| `parse_progress_ignores_comments_and_blanks` | Non-task lines skipped |
| `derive_status_marks_committed_tasks_done` | Git log updates status; first undone → Doing |
| `to_formatted_string_contains_current_and_next` | Output contains expected sections |

Additional tests to add:

- `load_returns_none_when_no_progress_file` — `ProjectProgressContext::load` on a
  tmpdir without `.ai/progress.md` returns `None`.
- `load_uses_written_status_when_git_unavailable` — in a non-git tmpdir, statuses
  from the file are preserved.
- `to_formatted_string_max_lines` — output for a maxed-out context is ≤ 100 lines.

### Manual testing

1. Create `.ai/progress.md` in any project, set one task to `doing`.
2. Open Warp AI — confirm the progress block appears in the conversation
   context (visible in the "context" chip or by asking "what are you working on").
3. Commit with `[N]` in the message; reopen AI — confirm the task advances.
4. Open a second Warp window in the same directory — confirm both show the
   same current task.
5. Remove `.ai/progress.md` — confirm AI session is unaffected (invariant 1).

## Risks and mitigations

| Risk | Mitigation |
|------|------------|
| `git log` adds latency to every AI query | Command runs synchronously but is capped at 50 commits; typical wall time < 20 ms |
| `progress.md` may be large | Line limits on every section cap total output at ≤ 100 lines |
| Regex may reject valid task lines | Regex is liberal with whitespace; ill-formed lines are silently skipped without affecting valid ones |
| Two `ProjectRules` entries (WARP.md + progress) may confuse the model | Both are standard rule files; the server already supports multiple `ProjectRules` entries |

## Follow-ups

- Watch `.ai/progress.md` for changes with `DirectoryWatcher` to avoid the
  git subprocess on every query (follow-up perf improvement).
- Add a `/warp-progress` slash command to render the current progress block
  in the terminal for quick inspection.
- Consider a `warp_context_max_lines` setting to let power users raise the
  100-line cap.
