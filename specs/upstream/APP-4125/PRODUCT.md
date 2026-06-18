# APP-4125: Unified Git Operations Dropdown
## Summary
Collapse the code review header's git operations dropdown into a stable, three-item menu that always exposes Commit / Push (or Publish) / Create PR, with per-item disabled states derived from the current branch state. The dropdown no longer offers chained "Commit and push" / "Commit and create PR" items — those chains remain available as intent selections inside the commit dialog.
## Problem
Before this change the git operations dropdown's contents shifted dramatically with the branch state: in Commit mode it listed "Commit", "Commit and push", and "Commit and create PR"; in Push mode it listed "Commit" (greyed), "Push", and "Create PR". Users had to re-scan the menu on each state change to find the action they wanted, and the chained items duplicated what the commit dialog already exposes via its intent selector. On branches with no upstream the dropdown disappeared entirely even though Publish was the one thing the user wanted to do.
We want one consistent dropdown shape where the three operations always appear in the same order and position, the user learns the menu once, and state merely toggles which items are enabled.
## Goals
- Stable dropdown composition across Commit and Push primary modes: Commit, Push-or-Publish, PR/Create PR — in that order.
- Per-item disabled states derived from the branch state (uncommitted changes, upstream, local commits, on main, existing PR).
- Drop the "Commit and push" and "Commit and create PR" dropdown items. Those chained intents remain available inside the commit dialog via the segmented intent selector.
- Send-to-remote item labels and icons swap between "Push" (`ArrowUp`) and "Publish" (`UploadCloud`) based on whether the branch has an upstream — both inside the commit dialog's intent selector and in the header dropdown.
- Publish primary mode keeps its own primary button but hides the chevron entirely, because the dropdown has nothing useful to show until the branch has been published.
- Inside the commit dialog, the confirm button is a static "Confirm" with no icon; the intent selector above it is the only UI that reflects which chain will run.
## Non-goals
- Changing the commit dialog's layout, fields, or async behavior beyond the confirm button's copy and the push/publish label on the middle intent.
- Surfacing new git operations (e.g. fetch, rebase) in the dropdown.
- Changing the header's primary button logic beyond Publish keeping its chevron hidden.
- Removing the commit-and-push / commit-and-create-PR chains themselves; those still run when selected inside the dialog.
## Figma
https://www.figma.com/design/T2CtyXgIdjtrLfC03K1n1H/Code-review-2.0?node-id=6138-21140&m=dev
## User experience
### Header git operations button
The primary action button (label + icon + click behavior) is unchanged from before: it reflects the computed `PrimaryGitActionMode` — Commit, Push, Publish, CreatePr, or ViewPr. The chevron immediately to the right of the primary button toggles the dropdown when the primary mode is Commit or Push. In Publish, CreatePr, and ViewPr modes the chevron is hidden entirely (nothing in a dropdown would be useful in those states).
### Dropdown contents
When the chevron is available and opened, the dropdown always lists three items in the same order:
1. **Commit** — `Icon::GitCommit`, opens the commit dialog.
2. **Push** or **Publish** — depending on whether the branch has an upstream. "Push" uses `Icon::ArrowUp` and opens the push dialog. "Publish" uses `Icon::UploadCloud` and opens the publish dialog. The label and action swap together.
3. **PR item** — either "PR #N" (linking to the existing PR) or "Create PR" (opens the create-PR dialog).
The enabled/disabled rules:
**Commit mode** (uncommitted changes exist, primary = Commit):
- Commit: enabled.
- Push / Publish: enabled iff there are local unpushed commits. Label is "Push" if upstream exists, else "Publish". Disabled otherwise (nothing to send).
- PR item: "PR #N" (enabled) if a PR exists; else "Create PR" disabled on main or when the branch has no upstream; otherwise "Create PR" enabled. Uncommitted changes do not block creating a PR because the PR is based on already-pushed commits.
**Push mode** (no uncommitted changes, local unpushed commits exist on a tracked branch, primary = Push):
- Commit: always disabled (nothing to commit).
- Push: always enabled (this is the primary action).
- PR item: same rules as Commit mode.
**Publish, CreatePr, ViewPr modes**: chevron hidden, dropdown not rendered.
### Commit dialog intent selector
The segmented intent selector inside the commit dialog still offers three buttons stacked vertically:
1. **Commit** — `Icon::GitCommit`. Always present.
2. **Commit and push** / **Commit and publish** — label and icon flip based on `has_upstream` at dialog-open time. "Commit and push" uses `Icon::ArrowUp`; "Commit and publish" uses `Icon::UploadCloud`. The underlying chain runs `run_commit` + `run_push` (which always uses `--set-upstream`) in either case.
3. **Commit and create PR** — `Icon::Github`. Present only when `allow_create_pr` is true (no existing PR on this branch and not on main). Omitted otherwise.
The initially-selected button is always "Commit". Clicking any of the three changes which chain will run but does not change the confirm button's label or icon.
### Commit dialog confirm button
- Label: static "Confirm" regardless of which intent is selected. No icon.
- Loading state: the label changes to "Committing…" while any of the three chains are in progress. It does not tell the user which chain is running mid-flight; the success toast communicates what actually ran.
- The segmented intent selector above the confirm button is the sole UI that communicates which chain will run on click.
### Primary button click behavior
- Primary = Commit: opens the commit dialog (which defaults to the plain `CommitOnly` intent).
- Primary = Push: opens the push dialog in push mode.
- Primary = Publish: opens the push dialog in publish mode.
- Primary = CreatePr: opens the create-PR dialog.
- Primary = ViewPr: opens the PR URL in the browser.
These are unchanged from before.
### State invariants
- The primary button's label, icon, and click action must match the dropdown's middle item label, icon, and action at all times when the dropdown is visible. In practice they're derived from the same computed state (`PrimaryGitActionMode`, `has_upstream`).
- The dropdown's shape (number and position of items) is identical in Commit and Push modes. Only disabled state and the middle item's label/icon/action differ.
- Disabled items still render with the same label, icon, and position as when they're enabled — users learn the menu once.
- The commit dialog's "Commit and push" / "Commit and publish" label is determined at dialog-open time from `has_upstream`. It does not update mid-dialog if the upstream changes externally.
## Success criteria
1. On a branch with uncommitted changes and an upstream, clicking the chevron shows Commit (enabled) / Push (enabled iff unpushed commits exist) / Create PR (enabled iff not on main, PR link if one exists).
2. On a branch with uncommitted changes and no upstream, the dropdown's middle item reads "Publish" with `Icon::UploadCloud`; "Create PR" is disabled.
3. On a clean working tree with unpushed commits, the dropdown shows Commit (disabled) / Push (or Publish, based on upstream) (enabled) / PR item. The primary button shows Push or Publish correspondingly.
4. In Publish primary mode, the chevron is not rendered; the dropdown is unreachable.
5. In CreatePr and ViewPr primary modes, the chevron is not rendered.
6. Opening the commit dialog with an upstream shows "Commit and push" as the middle intent. With no upstream, shows "Commit and publish".
7. Selecting any intent in the commit dialog keeps the confirm button label at "Confirm" and removes any icon from it. The segmented selector highlights the chosen intent.
8. Clicking "Confirm" runs the chain corresponding to the currently-selected intent. The loading label reads "Committing…" regardless of which chain is running. The success toast reflects what actually ran: "Changes successfully committed.", "Changes committed and pushed.", or the PR-created toast.
9. The commit dialog's third intent ("Commit and create PR") is omitted entirely when the branch already has a PR or we're on the repo's main branch.
10. Clicking "Commit" in the dropdown opens the commit dialog with the plain `CommitOnly` intent pre-selected. Users can switch to a chained intent inside the dialog.
11. `CodeReviewAction::CommitAndPush` and `CodeReviewAction::CommitAndCreatePr` are removed; no menu item or keyboard shortcut dispatches them.
## Validation
- On a tracked branch with uncommitted changes and unpushed commits, click the chevron: verify Commit, Push, Create PR are all enabled (Create PR enabled iff not on main / no existing PR).
- Delete the upstream (or test on a freshly-created local branch) and repeat: verify the middle item says "Publish" and uses the cloud icon; Create PR is disabled.
- On a clean tree with unpushed commits, verify the chevron still shows; Commit is disabled; Push/Publish and PR behavior match the rules above.
- On a branch with a clean tree and no unpushed commits on a non-main branch that has an upstream but no PR, verify the primary button reads "Create PR" and the chevron is hidden.
- On a branch with no upstream and nothing to commit but something to publish (e.g. fresh main with a commit), verify primary = Publish, chevron hidden.
- Open the commit dialog on a tracked branch: verify middle intent is "Commit and push" + ArrowUp. Switch to "Commit and push": confirm button still reads "Confirm" with no icon.
- Open the commit dialog on a branch with no upstream: verify middle intent is "Commit and publish" + UploadCloud.
- On a branch with an existing PR or on main, verify "Commit and create PR" is omitted from the intent selector.
- Confirm each of the three chains completes and shows the correct success toast; loading label is always "Committing…".
- Verify `CommitAndPush` and `CommitAndCreatePr` no longer exist as top-level `CodeReviewAction` variants.
## Open questions
- Should the chevron be shown in CreatePr and ViewPr modes as a way to access a "push again" / "commit more" escape hatch? Current decision: no, the primary button is the expected path in those modes, and Commit/Push are reachable via the header's action menu or terminal.
- Do we want to change the intent-selector label from "Commit and push" to something shorter now that it's stacked above a neutral "Confirm" button? (No change proposed here.)
