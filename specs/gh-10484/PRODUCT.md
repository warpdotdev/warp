# Git source control in the Tools panel

## Summary

Add a Git source-control tab to Warp's Tools panel so developers can inspect and organize local working-tree changes and browse repository history without leaving Warp. The panel follows the information architecture of VS Code's Source Control view while using Warp's existing design system and Code Review/commit workflows.

Related issues: [#10484](https://github.com/warpdotdev/warp/issues/10484), [#8542](https://github.com/warpdotdev/warp/issues/8542).

## Goals

- Make staged, unstaged, untracked, deleted, renamed, and conflicted files visible at a glance.
- Support file-level and bulk staging/unstaging from the panel.
- Make recent branch, tag, remote, merge, and commit history available below the change list.
- Reuse Warp's existing Code Review and commit UI instead of adding another commit-message input.

## Non-goals

- Replacing Warp's Code Review diff renderer or its existing commit/push/create-PR dialog.
- Hunk- or line-level staging in this first version.
- Destructive discard/reset actions without Warp's existing Code Review confirmation flow.
- Remote-session Git mutation. The first version operates on local repositories in native Warp builds.

## Figma

Figma: none provided. The request includes screenshots of VS Code's Source Control panel and Warp's current Tools panel as design references.

## Behavior

1. When the `GitToolsPanel` feature is enabled in a native build, the Tools panel shows a fifth tab after Project Explorer, Agent conversations, Global search, and Warp Drive. The tab uses Warp's Git branch/source-control icon and a "Source control" tooltip.

2. Selecting the tab opens the source-control view without closing or replacing the active terminal, editor, Code Review panel, or other pane.

3. The selected Tools tab is preserved in a window snapshot and restored when the window is restored. Restoring a snapshot from a build that did not contain this tab continues to work.

4. The view follows the active tab's focused local Git repository. Changing tabs, focusing a pane in another repository, or changing the focused terminal's repository updates the view to the newly focused repository.

5. When the active pane group contains multiple local repositories or worktrees, the view exposes a repository selector. Selecting a repository changes both the change list and history to that repository without changing the terminal's working directory.

6. Repository labels use the repository directory name as the primary label and retain the full path in the accessible/hover description so same-named repositories remain distinguishable.

7. When no local repository is available, the panel shows a stable empty state explaining that Source control becomes available after opening or entering a local Git repository. It does not show stale data from the previously selected repository.

8. When the focused repository is remote, the panel explains that remote source-control operations are not supported by this first version; it does not attempt local Git commands against the remote path.

9. While repository state is loading, the panel shows a loading state. A refresh already in progress is not duplicated by repeated refresh clicks.

10. If a Git command fails, the panel shows a concise error state or banner with a retry/refresh affordance. The error includes enough context to identify the repository and failed operation but never logs file contents, credentials, or remote authentication secrets.

11. The top portion of the view displays the current branch or detached-HEAD identifier and the repository's upstream ahead/behind counts when available.

12. Working-tree entries are grouped into collapsible sections:
    - `Merge Changes` for conflicted entries.
    - `Staged Changes` for index changes.
    - `Changes` for tracked working-tree changes that are not staged.
    - `Untracked Changes` for files not yet tracked by Git.

13. Empty sections are omitted. When all sections are empty, the change area shows a "No changes" state while the history area remains usable.

14. A path that has both staged and unstaged edits appears in both `Staged Changes` and `Changes`, because each row represents a different Git state that can be acted on independently.

15. Every file row shows:
    - The file name.
    - Its repository-relative parent path when needed for disambiguation.
    - A status letter and semantic label for added, modified, deleted, renamed/copied, untracked, or conflicted state.
    - The previous path for a rename/copy in the row's hover/accessibility description.

16. File rows are sorted predictably by repository-relative path within each section. Paths with non-UTF-8 bytes or unusual whitespace do not corrupt adjacent entries; entries that cannot be represented safely are skipped with a non-fatal warning.

17. Selecting a changed-file row opens Warp's existing Code Review view for that repository. The source-control panel remains open so the user can continue navigating changes.

18. An unstaged or untracked file row has a Stage action. Activating it stages exactly that path, refreshes the panel after Git completes, and surfaces any failure without dropping the previous successful snapshot.

19. A staged file row has an Unstage action. Activating it removes exactly that path from the index while preserving its working-tree content, refreshes the panel after Git completes, and handles repositories with and without an existing `HEAD` commit.

20. Each non-empty section exposes the appropriate bulk action: Stage All for unstaged/untracked changes and Unstage All for staged changes. Bulk actions affect the complete applicable section, including deletions and renames, and are disabled while another source-control mutation is running.

21. Conflicted files are never silently staged as part of an individual row action. The panel routes users to Code Review/merge resolution for conflicts; a later explicit Stage All may stage only after Git considers the conflicts resolved.

22. The panel never discards working-tree data directly. Users who need to discard changes use Warp's Code Review discard action, which owns the existing confirmation and optional stash behavior.

23. The lower portion is a collapsible `History` section. It shows recent commits from local branches, remote-tracking branches, and tags as a graph ordered consistently with Git's date/topology ordering.

24. Each history row shows a graph node/lanes, the commit subject, an abbreviated commit hash, and compact labels for `HEAD`, local branches, remote branches, and tags that point at that commit.

25. History initially loads a bounded page. Reaching the end or activating `Load more` appends the next page without replacing the existing rows or changing the current scroll position.

26. A manual Refresh reloads both working-tree state and the first history page from local Git state. It does not implicitly fetch from a remote, prompt for credentials, or mutate refs.

27. Filesystem/index/ref changes detected for the selected repository trigger a debounced refresh. Results from an older repository or request are ignored if the user switches repositories while that request is running.

28. Switching repositories resets history pagination, selection, and transient errors for the old repository. Returning to a repository may use a current cached snapshot, but stale results must be visibly refreshed.

29. Collapsed/expanded state for the change groups and History remains stable across ordinary refreshes in the same repository. Repository switches may use the default expanded state.

30. Keyboard focus can enter the panel, traverse repository controls, section headers, file rows, row actions, history rows, and Load more, then return to the rest of Warp. All icon-only actions have text tooltips and accessible names.

31. Colors, typography, row hover/selection, borders, buttons, scrollbars, and empty/error states use Warp theme tokens and existing UI abstractions. The panel does not hard-code VS Code colors or change shared button themes.

32. Large repositories remain responsive: Git commands run off the UI thread, the history list is virtualized or otherwise bounded by pagination, and refreshes are coalesced rather than spawning unbounded concurrent Git processes.
