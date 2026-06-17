# APP-4218: Git operations dialogs compare against the branch's actual parent
## Summary
The Push and Create PR dialogs must scope their "Included commits" / "Changes" previews to the commits and changes on top of the current branch's *actual* parent branch — the branch it was created off of. Today the dialogs fall back to comparing against the repo's main branch when there's no upstream, so a branch-off-a-branch surfaces every inherited commit as if it were new work.
## Problem
When a user creates `feature-b` off `feature-a` (no upstream yet) and opens the Push / Publish dialog, the "Included commits" list shows every commit between `main` and `HEAD` — including all commits inherited from `feature-a`. The same thing happens in the Create PR dialog's "Changes" list, its AI-generated title/description input, and the resulting `gh pr create` command (which targets the repo default branch). The user only wants to publish / open a PR for the work they actually did on `feature-b`.
The correct base is a property of the branch itself (what it was branched from), not a user choice. The dialogs should detect that base from the repo and use it directly.
## Goals
- The Push / Publish and Create PR dialogs compare against the current branch's actual parent branch, not unconditionally against main.
- Parent-branch detection is automatic, simple, and independent of the code review pane's diff-mode selector.
- The Create PR dialog targets the detected parent via `gh pr create --base <parent>`.
- Fall back to the repo's main branch when detection can't produce a parent, preserving today's common-path behavior.
## Non-goals
- Letting the user override the detected parent (follow-up if needed).
- Reflecting the detected parent anywhere outside the Push and Create PR dialogs.
- Changing the Commit dialog's behavior beyond its `Commit and create PR` chain.
## Figma
Figma: none provided (text/list-only change inside existing dialog chrome).
## Behavior
1. **Parent branch.** The current branch's parent is the local or remote-tracking branch whose tip is an ancestor of `HEAD` and is *closest* to `HEAD` — i.e. `git merge-base --is-ancestor <candidate> HEAD` succeeds and `git rev-list --count <candidate>..HEAD` is smallest among the candidates. The current branch is excluded from the candidate pool. If no candidate qualifies, the parent is the repo's detected main branch.
2. **Push / Publish dialog — included commits.** The "Included commits" list shows the commits in `<parent>..HEAD`. When there are no commits ahead of the parent, the list is empty and the primary Publish button is disabled.
3. **Create PR dialog — changes list.** The "Changes" list shows the files and per-file stats from `<parent>..<end_ref>`, where `<end_ref>` is `origin/<current_branch>` if that remote ref exists, otherwise `HEAD` — same end-ref pattern as today.
4. **Create PR dialog — AI inputs.** The diff fed into AI title / description generation and the branch commit-message list also use `<parent>..<end_ref>`.
5. **Create PR dialog — PR target.** `gh pr create` is invoked with `--base <parent>` so the PR targets the parent branch. If the parent ref is a remote-tracking ref (e.g. `origin/feature-a`), the `origin/` prefix is stripped before passing to `gh`. Applies to both the AI-generated-content path and the `--fill` fallback.
6. **Detection timing.** Parent detection runs whenever the dialog's helpers need it; results are not cached long-term. Rebases and new branches are picked up on the next open.
7. **Fallback errors.** If detection fails outright (not a git repo, git call errors) the dialogs use the main branch. If `gh pr create --base <parent>` is rejected by GitHub (e.g. the parent isn't pushed to origin), the user sees the existing generic "Git operation failed." toast; the dialog does not silently retry without `--base`.
8. **Commit dialog.** The Commit dialog itself is unchanged. Its `Commit and create PR` chain uses the detected parent for the final PR step (same rule as §5).
9. **Independence from the diff-mode dropdown.** The pane's diff-mode dropdown is unrelated to detection. Changing it doesn't change the dialogs' previews, and detection doesn't change the dropdown.
## Success criteria
1. On `feature-b` branched from `feature-a` (no upstream on either), the Publish dialog shows only the commits added on `feature-b`, and the Create PR dialog's Changes list shows only files changed on `feature-b`.
2. Confirming Create PR in that setup runs `gh pr create --base feature-a …` and the resulting PR targets `feature-a`.
3. On a fresh branch off `main` (no upstream), the Publish dialog shows `main..HEAD` — same commits as today.
4. After rebasing `feature-b` onto `main`, the next dialog open detects `main` as the parent and shows the rebased range.
5. If no branch is an ancestor of `HEAD`, detection falls back to `main` — no error.
6. The Commit dialog's `Commit and create PR` chain targets the detected parent.
7. The pane's diff-mode dropdown and the dialogs' previews are independent: changing one does not change the other.
