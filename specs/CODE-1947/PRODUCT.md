# CODE-1947: PR stacking UX in the code review pane

Status: Draft. The assumptions in this document require requester approval.

## Summary
Warp will show the structure of an existing GitHub-native pull request stack inside the desktop client's code review pane. A user can move between the focused diff for each pull request without checking out another branch or changing the working tree. Version 1 is a local, read-only review experience. Stack creation and mutation remain GitHub or CLI workflows.

## Assumptions pending confirmation
The requester did not answer the product interview synchronously. These defaults are assumptions, not requester decisions.

- **A1 — Read path first (load-bearing):** V1 discovers, displays, and navigates existing stacks. It does not create or mutate them. Reversing this assumption adds destructive workflows, conflict recovery, and GitHub write APIs to V1.
- **A2 — No checkout (load-bearing):** Layer switching uses a read-only commit-range diff and never checks out the layer. Reversing this assumption removes the two-revision diff requirement but adds working-tree mutation, dirty-tree handling, and branch recovery.
- **A3 — Layer diff only:** The default and only V1 stack diff is parent-to-layer.
- **A4 — Preserve local review:** Working-tree and agent "Review Changes" flows remain the default and are separate from stack browsing.
- **A5 — Local first:** Remote and SSH parity is deferred.
- **A6 — GitHub native only:** Graphite and inferred stacks are deferred.
- **A7 — Human review first:** Agent stack creation and management are deferred.
- **A8 — Warp-native map:** V1 uses the compact vertical map defined below because no Figma or approved mock was provided.
- **A9 — Per-layer comments:** Comments are isolated by working-tree context or pull request number.
- **A10 — Ephemeral selection:** The selected stack and layer are not persisted across application restarts.

## Problem
The code review pane treats the checked-out branch as one independent pull request. A stacked change has more context: each pull request depends on the layer below it, each layer has its own focused diff, and reviewers need to understand and navigate that order.

Opening every layer on GitHub loses the local review experience. Checking out every layer inside Warp would preserve local rendering, but it would also mutate the user's repository and could conflict with uncommitted work. The pane needs a safe review model that exposes stack context without taking ownership of the user's branch workflow.

## Goals
- Show when the current branch's pull request belongs to a GitHub-native stack.
- Show the stack order, trunk, pull request identity, and basic state inside the code review pane.
- Let the user review any layer's parent-to-layer diff without checking out that layer.
- Keep working-tree review and agent "Review Changes" behavior unchanged.
- Keep comments isolated to the working-tree review or pull request layer where they were created or imported.
- Degrade to today's single-PR behavior when stack data is unavailable.

## Figma
Figma: none provided. The linked X post is product framing, not a design source of truth.

## V1 scope
V1 includes:
- Discovery of the GitHub-native stack containing the pull request for the checked-out branch.
- A compact stack control and vertical stack map in the code review pane header.
- Read-only navigation among pull request layer diffs.
- Per-layer comment isolation.
- Opening any layer's pull request on GitHub.
- Local repositories only.
- A dedicated feature flag for rollout.

## Behavior
### Discovery and default state
1. Warp attempts stack discovery only when all of these conditions are true:
   - The PR stacking feature flag is enabled.
   - The code review pane is showing a local GitHub repository.
   - The checked-out branch has a pull request.
   - GitHub CLI authentication is available through the pane's existing `gh` integration.

2. Stack discovery does not delay the initial working-tree diff. The pane opens in its existing "Uncommitted changes" mode while GitHub metadata loads.

3. When the current pull request belongs to a stack with two or more pull requests, the pane shows a stack control in the header. The control identifies the stack size and the current branch's position, for example, "Stack · 2 of 4."

4. When the current pull request does not belong to a stack, Warp does not show a stack control. The pane behaves exactly as it does without this feature.

5. Warp does not infer a stack from branch ancestry, pull request base branches, branch names, Graphite metadata, or local `gh stack` state. Only GitHub's native stack membership activates the V1 experience.

### Stack map
6. Activating the stack control opens a compact vertical stack map anchored to the code review header.

7. The map renders the dependency chain with the trunk at the bottom, the bottom pull request immediately above the trunk, and higher pull requests above their parents. This matches the conceptual order used by GitHub while fitting Warp's narrow pane.

8. The trunk row shows the stack's base branch name and is not selectable as a pull request layer.

9. Each pull request row shows:
   - Pull request number.
   - Pull request title.
   - Head branch name.
   - State: draft, open, merged, or closed.
   - A "Current branch" marker when its head branch is checked out.
   - A selected treatment when its diff is currently displayed.

10. The map remains usable when titles or branch names are long. Text truncates with a tooltip that exposes the complete value. The pull request number and state remain visible.

11. The map supports pointer and keyboard use:
   - `Enter` or `Space` opens the map from the stack control.
   - Arrow keys move focus between pull request rows in visual order.
   - `Enter` selects the focused pull request.
   - `Escape` closes the map and returns focus to the stack control.
   - Focus, selection, "Current branch," and pull request state do not rely on color alone.

### Reviewing a layer
12. Selecting a pull request enters stack-layer review mode. Warp loads the exact GitHub pull request diff from the base commit reported for that pull request to its head commit. The bottom pull request normally targets the stack trunk, and each unmerged higher pull request normally targets the layer immediately below it. Warp honors GitHub's reported base after a partial merge retargets or rebases the remaining layers.

13. The selected layer's focused diff is the only stack diff mode in V1. Warp does not add a cumulative "entire stack to trunk" diff in V1.

14. Selecting a layer never:
   - Checks out a branch.
   - Changes `HEAD`.
   - Changes the index or working-tree files.
   - Stashes, discards, rebases, fetch-merges, or pushes user work.

15. A stack-layer diff is read-only, including when the selected layer is the checked-out branch. The user can navigate files, search, copy, add review comments, attach diff context, and submit comments to an agent through existing review flows. The user cannot edit or save files from this historical commit-range view.

16. While a stack layer is selected:
   - The header identifies the selected pull request and its parent-to-layer range.
   - The primary pull request action opens the selected pull request on GitHub.
   - Working-tree mutation actions, including discard and save, are hidden or disabled.
   - Commit, push, publish, and create-PR actions are not presented as actions on the selected layer.

17. Selecting another row replaces the displayed diff with that layer's diff. Warp keeps the map open until selection starts, then closes it and moves focus to the loaded review.

18. While a layer loads, the previous diff is replaced by a loading state that identifies the target pull request. Rapidly selecting multiple layers displays only the result for the last selection.

19. If Warp cannot load a selected layer, it shows an inline error with "Retry" and "Open on GitHub" actions. The error states that Warp did not change the working tree. Other layers remain available.

### Working-tree coexistence
20. "Uncommitted changes" remains the default and remains available in the existing diff selector. Selecting it exits stack-layer review mode and restores the checked-out branch's working-tree diff.

21. Exiting stack-layer review restores the existing working-tree review state, including file expansion, scroll position when available, and its draft comments. It does not check out the layer that was last viewed.

22. Agent "Review Changes" continues to open the working-tree review. It does not automatically enter stack-layer review, even when the current branch belongs to a stack.

23. The existing create-PR workflow is unchanged in V1. Warp does not automatically add a newly created pull request to a stack and does not change its base based on the displayed stack layer.

### Comments
24. The working-tree review and each pull request layer have independent comment sets.

25. A native draft comment created on a stack layer is associated with that pull request number and the base/head commit pair used when the comment was created. It is hidden when another layer or the working-tree review is selected and restored when the user returns.

26. Imported GitHub comments are associated with the pull request they came from. Warp never displays a comment from one pull request on another layer solely because the file path and line match.

27. When a selected pull request's head changes, Warp reloads the layer and repositions comments using the pane's existing relocation behavior. Comments that cannot be safely relocated remain in that layer's comment list and are marked outdated.

28. Switching layers, refreshing stack metadata, closing the stack map, or returning to "Uncommitted changes" never submits, deletes, or moves a draft comment.

29. Comment persistence does not expand in V1. Comments keep the same application-lifetime behavior they have today; they are not written to the pane persistence table.

### Refresh, errors, and restore
30. Warp refreshes stack metadata with the current pull request metadata lifecycle: on initial discovery, current-branch change, explicit git-state refresh, completion of a relevant `gh` operation, and the existing periodic refresh.

31. A refresh updates pull request titles, states, order, and commit identities. If the selected layer still exists and its commit identities are unchanged, Warp keeps the current diff and comments visible.

32. If the selected pull request leaves or dissolves from the stack, Warp exits stack-layer review and returns to "Uncommitted changes." It preserves that pull request's draft comments in memory in case it reappears during the application session and shows a non-blocking notice.

33. Missing `gh`, missing authentication, network failures, insufficient permissions, a GitHub Stacks API `404`, and repositories without the public-preview feature do not block normal code review. Warp hides the stack control and preserves today's single-PR experience.

34. A successful stack lookup with no matching stack is treated as "not stacked," not as an error.

35. V1 does not persist the selected stack or layer. Restoring a persisted code review pane restores the repository and opens "Uncommitted changes," then performs fresh pull request and stack discovery.

36. Remote and SSH code review panes do not show the stack control in V1. Their existing pull request and diff behavior is unchanged.

## Later scope
- Create a new stack or add a pull request to the top of an existing stack.
- Initialize and submit a local stack through an explicit `gh stack` workflow.
- Cascading sync or rebase, with conflict handling and working-tree safety checks.
- Merge one or more layers through GitHub's asynchronous stack merge API.
- Unstack or reorder layers.
- A cumulative stack-to-trunk diff.
- Remote and SSH code review parity.
- Agent-created or agent-managed stacks, coordinated with REMOTE-330.
- Inferred or Graphite-managed stacks.
- Persisted stack-layer selection if user research shows that restore-to-layer is valuable.

## References
- [CODE-1947](https://linear.app/warpdotdev/issue/CODE-1947/spec-pr-stacking-ux-in-the-code-review-pane)
- [GitHub: About stacked pull requests](https://docs.github.com/en/pull-requests/get-started/about-stacked-prs)
- [GitHub REST API: Stacked pull requests](https://docs.github.com/en/rest/pulls/stacks)
- [GitHub CLI: `gh stack`](https://docs.github.com/en/pull-requests/reference/stacked-prs-cli-commands)
- [Product framing reference](https://x.com/charlieholtz/status/2086167071696789990)
