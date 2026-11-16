# APP-4188: Tech Spec
## Context
The git-ops dialog spawns `git` and `gh` subprocesses through `run_git_command` (`app/src/util/git.rs:12`) and `run_gh_command` (`app/src/util/git.rs:663`). Neither sets `PATH` on the child — `run_gh_command` has a macOS-only hardcoded `/opt/homebrew/bin:/usr/local/bin:` prefix, which only helps Homebrew users and misses MacPorts/Nix/etc.
On Finder/Dock launches, Warp inherits launchd's minimal `PATH`. `git` itself still works (Apple ships `/usr/bin/git`), but hooks `git` invokes — notably LFS `pre-push` → `git-lfs` — fail. `gh` also fails outside Homebrew-default layouts.
`LocalShellState::get_interactive_path_env_var` (`app/src/terminal/local_shell/mod.rs:145`) already captures the user's real interactive-shell `PATH` (runs `zsh -i -l` / bash / fish, caches the result). This is how LSP resolves binaries today (`persisted_workspace.rs:1119`), so it's already the repo's idiom.
## Proposed changes
1. **`util/git.rs`: add `run_git_command_with_env(repo, args, path_env: Option<&str>)`** that sets `PATH` on the child when `path_env` is `Some`. Keep `run_git_command(repo, args)` as a thin wrapper passing `None` — no ripple on existing call sites.
2. **Grow `run_gh_command`, `run_commit`, `run_push`, `create_pr`, `get_pr_for_branch`** to take `path_env: Option<&str>` and forward. Only hook-firing and `gh`-spawning wrappers touched; pure read-only `git` wrappers (`get_unpushed_commits`, `get_branch_commit_messages`, `get_diff_for_pr`, `get_branch_diff_entries`) are left alone.
3. **Remove the hardcoded Homebrew prefix from `run_gh_command`.** Callers that need `gh` findable now pass a captured interactive PATH. `HOMEBREW_NO_AUTO_UPDATE=1` stays.
4. **`git_dialog/{push,commit,pr}.rs::start_confirm`** capture the PATH before spawning:
   ```rust path=null start=null
   let path_future = interactive_path_future(ctx);
   ctx.spawn(async move {
       let path_env = path_future.await;
       run_push(&repo_path, &branch, path_env.as_deref()).await
   }, ...);
   ```
   Commit's `start_confirm` forwards through the commit → push → `create_pr_with_ai_content` chain; PR's `start_confirm` forwards into `create_pr_with_ai_content`, which forwards `path_env` into `create_pr` only. Pattern matches `persisted_workspace.rs::execute_lsp_task`.
5. **`diff_state.rs::refresh_pr_info`** captures PATH via a local `interactive_path_future` helper (same shape, `ModelContext<DiffStateModel>`-flavored) and forwards into `get_pr_for_branch` so the `PR #N` header badge works from Finder launches.
## Fallbacks
- Capture returns `None` (wasm / `LocalShellState` not loaded / shell errored) → callers forward `None` → subprocess uses inherited `PATH`. Behavior no worse than today.
- On Linux/Windows the same code path runs; no special-casing.
## Testing and validation
No unit tests — the plumbing is a thin `Option<&str>` forward and the real-world condition (launchd minimal `PATH`) isn't reproducible in a test harness. Manual validation per `PRODUCT.md` "Validation". UI regression coverage via `verify-ui-change-in-cloud`.
## Risks
- **First-click latency:** first git-op in a session awaits the shell capture (<1s typical). Absorbed by existing loading state. Prewarming on panel mount is an easy follow-up if it's ever user-visible.
- **Hook behavior change:** LFS hooks that silently no-op today (because `git-lfs` is missing) will actually run for Finder-launched Warp. Intended per PRODUCT.md.
## Follow-ups
- Prewarm `get_interactive_path_env_var` at code-review panel mount if first-click latency becomes a complaint.
- Plumb `path_env` through other `run_git_command` call sites (repo_metadata, drive sync, etc.) if they start hitting the same class of failure.
