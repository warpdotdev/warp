# Recursive file search in the file finder outside git repositories

GitHub: [warpdotdev/warp#13341](https://github.com/warpdotdev/warp/issues/13341)

## Summary
When the current working directory is not inside a git repository, the `⌘O` file finder can only list the immediate directory; it cannot fuzzy-find files in subdirectories. This feature adds an on-the-fly, `fzf`-style recursive search for non-git directories so users can find files across the current directory and its subdirectories, without eagerly indexing arbitrary parts of the filesystem.

Figma: none provided (behavior mirrors the existing in-repo finder; only the file source changes).

## Behavior
1. When the working directory is inside a detected git repository, the finder behaves exactly as it does today: recursive results are served from the repo-metadata index, honoring `.gitignore` and git-status ranking. This feature does not change that path.
2. When the working directory is not inside a detected git repository and the feature flag is enabled, the finder supports recursive fuzzy search across the current directory and its subdirectories. (Previously only the immediate directory was listed.)
3. Zero-state (empty query) is unchanged from today's non-git behavior: the finder shows the immediate directory's entries plus recently opened files. No recursive traversal happens until the user types a query.
4. As the user types, results stream in from an on-the-fly recursive walk rooted at the working directory; matches appear incrementally rather than all at once. Changing the query cancels the in-flight walk and starts a new one.
5. Traversal scope is the current working directory and below only. It never ascends into parent directories, `$HOME`, or the filesystem root.
6. Hidden files and dotfiles are included by default. A user setting controls this; when the setting is disabled, hidden files and directories are excluded from results. This mirrors the existing "show hidden files" behavior in the code file tree.
   - **Open question:** reuse the existing `show hidden files` setting that the file tree already uses, or introduce a finder-specific setting? Default proposal: reuse the existing one for consistency.
7. The directories `.git` and `node_modules` are always skipped, matching `fzf`'s default skip list.
8. `.gitignore` is not honored — gitignored files still appear in results. This matches `fzf`'s built-in walker default (per issue discussion, the chosen behavior).
9. Symlinked directories are not followed, to avoid cycles and runaway traversal into large linked trees. (This is the one deliberate divergence from `fzf`, which follows symlinks by default.)
10. Results are bounded so a very large or deep non-repo directory cannot hang the UI or exhaust memory:
    - At most 200 results are shown (same visible cap the finder already uses).
    - A soft scan/time budget bounds how much of the tree is walked per query.
    - When the cap or budget is reached, the finder shows partial results with no error, consistent with how the in-repo finder already surfaces truncated results.
11. Ranking, when there is a query: fuzzy match on the query (filename-weighted, same matcher as the in-repo finder), with ties broken by most-recently-opened, then shallower path depth, then shorter path. Git-status ranking does not apply outside a repo.
12. Selecting a result opens it exactly as the in-repo finder does (same open-in-editor action and behavior).
13. Directories that cannot be read (permission denied, deleted mid-walk) are skipped silently; the search continues over the rest of the tree.
14. The feature is gated behind a feature flag. With the flag off, the finder outside a git repository keeps today's immediate-directory-only behavior.
15. The recursive non-git search is consistent across every surface the file finder powers (the `⌘O` / `/open-file` finder and the `@` file context menu): the same query in the same directory produces the same results.
