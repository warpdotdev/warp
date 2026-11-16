# APP-3918: Git Operations Button in Code Review Header

## Summary

Add a context-aware git operations button to the code review header that surfaces the most relevant git action (Commit, Push, or Create PR) based on the current repository state. This builds on the header UI refactor in APP-3632.

## Problem

Users viewing diffs in the code review panel must leave the panel to commit, push, or create a PR. There is no inline way to advance changes through the git workflow from the code review context. This creates friction — especially for quick commit-and-push flows — and breaks the user's focus.

## Goals

- Surface the next logical git action directly in the code review header.
- Reduce the number of steps to go from reviewing diffs to committing / pushing / opening a PR.
- Provide a dropdown with related git operations so users aren't limited to only the primary action.

## Non-goals

- The actual commit, push, and create-PR dialogs are handled by child branches in the stack (`edward/commit-dialog`, `edward/push-dialog`, `edward/pr-dialog`). This branch only wires the button and dropdown; dialog implementation is out of scope.
- Staging individual files or hunks from the button (staging is done elsewhere).

## Figma

https://www.figma.com/design/T2CtyXgIdjtrLfC03K1n1H/Code-review-2.0?node-id=6138-21140&m=dev

## User Experience

### Button appearance

The git operations button is a split button rendered in the right section of the inner code review header (to the left of the file-nav and overflow buttons). It consists of:

1. **Primary action button** — an `ActionButton` with `SecondaryTheme`, `ButtonSize::Small`, an icon, and a label describing the current primary action.
2. **Chevron button** — a separate `ActionButton` with a `ChevronDown` icon adjoined to the primary button's right edge, opening a dropdown menu of related actions.

The two buttons use `AdjoinedSide::Right` (primary) and `AdjoinedSide::Left` (chevron) so they visually merge into one split button.

### Primary action mode

The button's label, icon, and click behavior are determined by the current `PrimaryGitActionMode`, which is recomputed whenever diff stats or unpushed-commit state changes:

1. **Commit** — when there are uncommitted changes (diff stats are non-empty).
   - Label: "Commit", Icon: `GitCommit`
   - Click dispatches `OpenCommitDialog`
   - Chevron visible

2. **Publish** — when there is no upstream tracking branch but there are local commits.
   - Label: "Publish", Icon: `UploadCloud`
   - Click dispatches `PublishBranch` (pushes and sets upstream in one step)
   - Chevron **hidden**

3. **Push** — when there are no uncommitted changes, branch has upstream, and there are unpushed commits.
   - Label: "Push", Icon: `ArrowUp`
   - Click dispatches `OpenPushDialog`
   - Chevron visible

4. **View PR** — when there is nothing to commit or push and a PR exists for the branch.
   - Label: "PR #N", Icon: `Github`
   - Click opens the PR URL in the browser
   - Chevron **hidden**

5. **Create PR** — when there is nothing to commit or push, branch has upstream, not on main, and no PR exists.
   - Label: "Create PR", Icon: `Github`
   - Click dispatches `OpenCreatePrDialog`
   - Chevron **hidden**

6. **Disabled Commit** (fallback) — when nothing is actionable (e.g. on main with no changes, or empty unpublished branch).
   - Label: "Commit", Icon: `GitCommit`, button disabled
   - Chevron **hidden**, no adjoined side (fully rounded)

### Dropdown menu

When the chevron is clicked, a dropdown menu appears anchored to the bottom-right of the button group. Menu items depend on the current mode:

**Commit mode:**
- Commit (icon: GitCommit)
- Commit and push (icon: ArrowUp)
- Commit and create PR (icon: Github)

**Push mode:**
- Commit (icon: GitCommit) — **disabled** (nothing to commit)
- Push (icon: ArrowUp)
- Create PR (icon: Github)

**Create PR / View PR / Publish / Disabled Commit modes:**
- Chevron is hidden; dropdown is never shown.

The dropdown closes when an item is selected or when the user clicks away. The chevron button shows an active/pressed state while the dropdown is open.

### Overflow menu (three-dot button)

When the `GitOperationsInCodeReview` flag is enabled and there are no changes, the overflow menu is hidden entirely. All overflow menu items (Discard all, Add diff set as context, Add comment) are gated on having changes.

### State transitions

The button updates reactively:
- When new git diffs arrive (`update_diff_state`), the mode is recomputed and the button label/icon/handler are updated.
- When the diff stats change (`apply_diff_stats`), the mode is recomputed.
- Transitions between modes are instant — no animation or intermediate states.

### Unpushed commits detection

To support the Push mode on new local branches (no upstream tracking branch), `get_unpushed_commits` falls back to comparing against the detected main branch (`main` or `master`) when `@{u}..HEAD` fails. This ensures the button correctly shows "Push" on a fresh branch with local commits.

### Feature flag

All git operations button UI is gated behind `FeatureFlag::GitOperationsInCodeReview`. When the flag is off, the button is not rendered and no new actions are dispatched.

## Success Criteria

1. When the code review panel shows uncommitted changes, the header displays a "Commit" button with a chevron dropdown.
2. When there are no uncommitted changes but unpushed commits exist (with upstream), the button reads "Push" with a chevron dropdown.
3. On a new local branch with no upstream but with local commits, the button shows "Publish".
4. When everything is pushed and a PR exists, the button shows "PR #N".
5. When everything is pushed, no PR exists, and not on main, the button shows "Create PR".
6. On main with no changes, the button shows a disabled "Commit" with no chevron.
7. The button transitions between modes automatically as the user commits or pushes (once dialogs are wired by child branches).
8. The dropdown displays the correct items for each mode, with "Commit" disabled in Push mode.
9. The button does not appear when `FeatureFlag::GitOperationsInCodeReview` is off.
10. The overflow menu is hidden when there are no changes.

## Validation

- Manual verification: toggle the feature flag and confirm the button appears/disappears.
- Modify a file in a repo, open the code review panel, and verify "Commit" is shown.
- Commit the change (via terminal) and verify the button switches to "Push".
- Push the branch and verify the button switches to "Create PR" with no chevron.
- On a fresh local branch with commits, verify "Push" is shown (not "Create PR").
- Open the chevron dropdown in each mode and verify correct menu items and disabled states.

## Open Questions

- Should compound actions ("Commit and push", "Commit and create PR") show a confirmation or progress indicator? (Deferred to dialog branches.)
