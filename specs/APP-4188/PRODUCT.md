# APP-4188: Git operations buttons respect the user's interactive shell PATH
## Summary
The code-review panel's git operations buttons (Commit, Push, Publish, Create PR, Commit and push, Commit and create PR) must succeed regardless of how Warp was launched. When opened from Finder/Dock, Warp inherits launchd's minimal `PATH`, which excludes Homebrew / MacPorts / Nix / asdf / etc. Pushes to LFS-enabled repos fail (LFS `pre-push` hook can't find `git-lfs`), and PR creation fails when `gh` isn't on `/usr/bin`.
## Behavior
1. Every subprocess the git-ops buttons spawn (`git commit`, `git push`, `gh pr view`, `gh pr create`, and hooks `git` invokes) runs with the `PATH` the user would see in an interactive login shell (`zsh -i -l` / equivalent). Launch method is irrelevant — Finder, Dock, terminal, `open -a`, etc.
2. On a Git LFS repo, Push / Publish / Commit and push / Commit and create PR succeeds whenever the user has `git-lfs` installed and reachable from their shell.
3. Create PR / Commit and create PR succeeds whenever the user has `gh` installed, authenticated, and reachable from their shell. The header's `PR #N` badge populates under the same conditions.
4. If a tool is genuinely missing, the existing friendly-error toasts still fire (e.g. "GitHub CLI (gh) not installed."). No regression on error messaging.
5. First git op per Warp session may pay a one-time shell-capture cost (<1s typically; bounded by `.zshrc` load). Absorbed by the existing loading state; no new UI. Subsequent ops reuse the cache.
6. If the shell capture fails, subprocesses fall back to the inherited `PATH` — no hang, behavior no worse than today.
7. No new settings or UI. Behavior is always-on.
## Validation
- Launch Warp from Finder with Homebrew-only `git-lfs`. On a LFS repo, Commit and push → succeeds.
- Uninstall `git-lfs`. Repeat → existing "Git operation failed." toast.
- Same setup with Homebrew-only `gh`, click Create PR → succeeds; header shows `PR #N`.
- Launch Warp from a terminal → all git ops behave as today.
