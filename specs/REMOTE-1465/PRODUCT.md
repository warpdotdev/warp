# Oz File-Edit Hooks for Snapshotting Non-Git-Tracked Files — Product Spec
Linear: [REMOTE-1465](https://linear.app/warpdotdev/issue/REMOTE-1465)
Figma: none provided

## Summary
When the Oz agent creates or edits a file during a cloud run, the file's absolute path is automatically added to the end-of-run snapshot declarations so the file is uploaded alongside the existing repo-diff snapshot. This closes the handoff gap for files the agent writes outside any git repo (or inside one but not visible to `git diff HEAD`), so the next execution of the same run can see them.

## Problem
The REMOTE-1332 snapshot pipeline captures workspace state via `git diff --binary HEAD` plus `git ls-files --others --exclude-standard` for every declared `.git` directory under the workspace. That captures tracked changes and untracked non-gitignored files inside a declared repo, but it silently loses:
- Files the agent creates or edits outside any git repo (e.g. `/tmp`, `$HOME`, or the workspace root before `git init`).
- Files inside a repo but below a `.gitignore` entry, which the git-diff path deliberately ignores.

After a cloud → cloud handoff, the next execution never sees those files, so agent work that deposits logs, scratch scripts, or generated artifacts outside a repo is effectively thrown away.

## Behavior

1. During a cloud Oz run, every time the agent successfully completes a file-edit tool call that creates or modifies a file, the file's absolute path is recorded in the snapshot declarations file as a `file` entry. The entry lands in the declarations file before the end-of-run upload reads it.

2. Each entry uses the existing REMOTE-1332 schema: `{"version":1,"kind":"file","path":"<absolute path>"}`. The declarations file is the same per-run file the existing pipeline reads (resolved via `$OZ_SNAPSHOT_DECLARATIONS_FILE`, otherwise `/tmp/oz/<task-id>/snapshot-declarations.jsonl`).

3. All recorded paths are absolute. If a tool-call result surfaces a relative path, it is resolved against the driver's `working_dir`. Paths that still cannot be made absolute are dropped with a WARN log and not recorded.

4. Within a single run, the same absolute path is never written more than once to the declarations file. Multiple edits to the same file, or a create followed by an edit, still produce only one `file` entry.

5. Appends happen incrementally as each tool call completes, not in a single batch at end of run. If the run crashes after some edits but before the upload step, any `file` entries that already landed on disk are still present for any recovery tooling that reads the declarations file.

6. File deletions are not recorded in v1. `RequestFileEditsResult::Success.deleted_files` is observed but ignored. Deletions of tracked files inside a declared repo are still captured by `git diff HEAD` in the existing pipeline.

7. Before upload, the snapshot pipeline drops `file` entries whose absolute path falls under any declared `repo` entry's path. This prevents double-uploading files that the repo's diff already captures. If the agent creates files and then later `git init`s a directory containing them, the end-of-run script emits a `repo` entry for the new directory, which causes the earlier `file` entries to be filtered out at gather time.

8. A `file` entry whose path is not under any declared repo is uploaded in full, exactly like an operator-authored `file` entry already would be today per REMOTE-1332.

9. The feature is inert when any of the following are true, matching the existing REMOTE-1332 gating:
   - `FeatureFlag::OzHandoff` is disabled.
   - The run has no associated task ID (purely local runs).
   - The run was started with `--no-snapshot`.
   In these cases, no declarations are written and no file-edit observations have any user-visible effect.

10. The hook only runs for the Warp Oz SDK driver. Third-party harnesses (e.g. Claude Code) do not participate in this mechanism; their file writes go through their own tools and are not observed (this can and will be fixed in follow-up PRs, using each agent's hook system to track file edits).

11. Writer failures (declarations file not writable, path normalization error, file system I/O error) are logged at WARN level and absorbed. The agent run, the current tool call, and the end-of-run snapshot upload all continue as if no file entry were recorded for that call.

12. Shell-driven writes (`cat > file`, `echo >>`, `tee`, `touch`, etc.) are not captured by this mechanism. Those files only appear in the snapshot when they happen to sit under a declared repo and the git-diff path catches them.

13. A gitignored file edited by the agent inside a declared repo is not captured by v1: it is dropped by the repo-overlap filter in (7) and git's diff path ignores it. This is a known v1 limitation; see "Open questions" below.

## Non-goals
- Capturing file deletions. The existing `file` declaration kind uploads bytes and cannot represent a tombstone. Tracked-file deletions are still covered by `git diff HEAD`.
- Capturing shell-driven file writes. Parsing arbitrary shell commands to infer file effects is brittle and out of scope.
- Adding start-of-run scanning. End-of-run repo scanning plus gather-time overlap filtering is enough to handle the "agent created files before `git init`" case; a redundant start-of-run scan is not added.
- Extending this behavior to third-party harnesses (Claude Code, etc.). Harness-specific hook support is a separate follow-up.
- Changing the existing snapshot declarations schema, the `snapshot-declarations.sh` script, or the Docker image.

## Open questions
- **Gitignored files inside a declared repo:** v1 drops any `file` entry whose path falls under a declared repo path, which means gitignored files inside that repo are silently lost. Should a follow-up tighten the overlap filter (e.g. use `git check-ignore -q` to keep `file` entries whose paths the repo's diff will not carry)?
- **Per-run size caps:** REMOTE-1332's `MAX_SNAPSHOT_FILES_PER_RUN = 100` already bounds total blobs. A very chatty agent could plausibly touch more than 99 distinct non-repo files in one run; any excess ends up as `skipped` in the manifest. Do we need a separate earlier-than-cap warning for operators when the tool-call writer approaches that ceiling?
