# Git source control in the Tools panel

## Summary

Add a Git source-control tab to Warp's Tools panel so developers can browse repository history without leaving Warp. Working-tree changes remain in Warp's existing Code Changes right panel.

Related issues: [#10484](https://github.com/warpdotdev/warp/issues/10484), [#8542](https://github.com/warpdotdev/warp/issues/8542).

## Goals

- Make recent branch, tag, remote, merge, and commit history available in the Tools panel.
- Show graph lanes, ref badges, and rich commit details in a compact history list.
- Keep repository selection, branch/tracking context, refresh, and pagination responsive.

## Non-goals

- Working-tree change lists, staging/unstaging, and discard/reset actions; the Code Changes right panel owns those workflows.
- A splitter or independently scrolling changes/history regions.
- Replacing Warp's Code Review diff renderer or its existing commit/push/create-PR dialog.
- Remote-session Git history. The first version reads local repositories in native Warp builds.

## Figma

Figma: none provided. The request includes screenshots of VS Code's Source Control panel and Warp's current Tools panel as design references.

## Behavior

1. When the `GitToolsPanel` feature is enabled in a native build, the Tools panel shows a fifth tab after Project Explorer, Agent conversations, Global search, and Warp Drive. The tab uses Warp's Git branch/source-control icon and a "Source control" tooltip.

2. Selecting the tab opens the source-control view without closing or replacing the active terminal, editor, Code Review panel, or other pane.

3. The selected Tools tab is preserved in a window snapshot and restored when the window is restored. Restoring a snapshot from a build that did not contain this tab continues to work.

4. The view follows the active tab's focused local Git repository. Changing tabs, focusing a pane in another repository, or changing the focused terminal's repository updates the view to the newly focused repository.

5. When the active pane group contains multiple local repositories or worktrees, the view exposes a repository selector. Selecting a repository changes the history to that repository without changing the terminal's working directory.

6. Repository labels use the repository directory name as the primary label and retain the full path in the accessible/hover description so same-named repositories remain distinguishable.

7. When no local repository is available, the panel shows a stable empty state explaining that Source control becomes available after opening or entering a local Git repository. It does not show stale data from the previously selected repository.

8. When the focused repository is remote, the panel explains that remote source-control history is not supported by this first version; it does not attempt local Git commands against the remote path.

9. While repository history is loading, the panel shows a loading state. A refresh already in progress is not duplicated by repeated refresh clicks.

10. If a Git command fails, the panel shows a concise error state or banner with a retry/refresh affordance. The error includes enough context to identify the repository and failed operation but never logs file contents, credentials, or remote authentication secrets.

11. The top portion of the view displays the current branch or detached-HEAD identifier and the repository's upstream ahead/behind counts when available.

12. Commit rows begin directly below the header or fixed error banner in one scrollable history region, separated from the header by a subtle theme border. There is no collapsible History header or splitter.

13. History shows recent commits from local branches, remote-tracking branches, and tags as a graph ordered consistently with Git's date/topology ordering.

14. Each single-line history row shows graph lanes, the commit subject, and compact labels for `HEAD`, local branches, remote branches, and tags. Hovering shows the author, relative or absolute timestamp, body, abbreviated hash, refs, and available short statistics.

15. History initially loads a bounded page. Activating `Load more` appends the next page without replacing the existing rows or changing the current scroll position.

16. A manual Refresh reloads the first history page from local Git state. It does not inspect the working tree, implicitly fetch from a remote, prompt for credentials, or mutate refs.

17. Repository metadata/ref changes detected for the selected repository trigger a debounced refresh. Results from an older repository or request are ignored if the user switches repositories while that request is running.

18. Switching repositories resets history pagination and transient errors for the old repository. Returning to a repository may use a current cached snapshot, but stale results must be visibly refreshed.

19. Keyboard focus can enter the panel, traverse repository controls, history rows, and Load more, then return to the rest of Warp. All icon-only actions have text tooltips and accessible names.

20. Colors, typography, row hover, borders, buttons, scrollbars, and empty/error states use Warp theme tokens and existing UI abstractions. The panel does not hard-code VS Code colors or change shared button themes.

21. Large repositories remain responsive: Git commands run off the UI thread, history is bounded by pagination, and refreshes are coalesced rather than spawning unbounded concurrent Git processes.
