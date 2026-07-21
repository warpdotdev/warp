# Project Explorer Directory Symlinks — Product Spec

## Summary

For a local macOS workspace, Project Explorer displays and traverses directory symbolic links using
their path inside the workspace, so users can browse and open the linked files without leaving Warp.
The behavior fixes [#11528](https://github.com/warpdotdev/warp/issues/11528) while remaining bounded
when links are broken, unreadable, outside the workspace, or cyclic.

## Goals

- Make a directory symlink usable from Project Explorer in the same place and under the same name
  as it appears on disk.
- Keep traversal finite and responsive for cyclic or very large linked directory graphs.
- Keep file-tree updates, selection, and open-file paths anchored to the workspace-visible symlink
  path.
- Preserve existing ignore, lazy-loading, and file-budget behavior.
- Keep this capability limited to user-initiated Project Explorer browsing, explicit path
  attachment, and file operations.

## Non-goals

- Add a new icon, badge, warning, or context-menu action for symbolic links.
- Change the behavior of symbolic links to files.
- Expose the target's parent directory or any sibling outside the linked subtree.
- Resolve permission failures, repair broken links, or change filesystem permissions.
- Make Project Explorer follow shell aliases, Finder aliases, or other non-filesystem shortcuts.
- Add linked target contents to repository search/indexing, automatically collected agent context,
  standing-query or skill discovery, or remote repository snapshots. This does not exclude the
  existing explicit **Attach as context** action described below.
- Change remote-backed, Windows, or Linux Project Explorer behavior; those surfaces remain as they
  are today.

## Figma

Figma: none provided. The issue contains screenshots of the current Warp result and the expected
expandable tree behavior; this change introduces no new visual treatment.

## Behavior

1. In a local macOS workspace, a symbolic link whose target can be identified as a directory
   appears in Project Explorer at the link's path and with the link's filename, in the same sorted
   position as an ordinary directory. Directory classification does not require enumerating the
   target's contents, so a directory whose contents are temporarily unreadable still appears as an
   unloaded, retryable alias.

2. The linked directory is loaded on demand. Before it is expanded, Project Explorer retains it as
   an unloaded directory, does not walk the target, and does not begin watching an external target.
   Expanding it reveals the target directory's children without eagerly walking the entire target.

3. Every descendant is presented under the link's workspace-visible path. For example, if
   `/workspace/vendor` links to `/shared/library`, the target file `/shared/library/src/lib.rs`
   appears and opens as `/workspace/vendor/src/lib.rs`. Project Explorer does not replace the link
   with `/shared/library` or expose `/shared`.

4. Opening, selecting, copying the path of, attaching as context, renaming, or deleting a
   descendant under a linked directory uses the workspace-visible path. The operating system
   continues to apply its normal symlink semantics to that explicit descendant operation; Warp
   does not silently redirect the operation to a different visible tree entry.

   **Attach as context** is available on an alias root and on visible file or directory descendants,
   just as it is on ordinary Project Explorer entries. It sends the repository-relative lexical
   path (for example, `vendor/src/lib.rs`) through the existing path-attachment flow. Selecting the
   action does not canonicalize the path, reveal the external target, recursively enumerate a
   directory, or add the linked subtree to automatically collected agent context.

   The directory-symlink root is a separate destructive boundary. Renaming the root renames only
   the symlink entry at its workspace-visible path; the target directory, its name, and its contents
   do not move or change. Deleting the root unlinks only the symlink entry and never recursively
   deletes or modifies the target directory or any target descendant. Rename/delete confirmation
   and error UI identify the workspace-visible alias path, not the canonical external target.

5. A link may target a directory outside the workspace. Only the target directory and its
   descendants are reachable through the link; the target's parent and siblings do not become
   Project Explorer roots.

6. A link may target a directory that is also reachable through an ordinary workspace path. Both
   entries remain visible: the ordinary path and each symlink path are separate Project Explorer
   locations, even though they resolve to the same filesystem objects.

7. Multiple directory links to the same target remain separate entries. Expanding, collapsing, or
   selecting one alias does not change the expansion, collapse, or selection state of another
   alias.

8. Traversal is cycle-safe across separate expansion actions. Warp remembers the canonical
   ancestry of an unloaded linked directory when it is added to the tree. If the user expands an
   `A → B → A` chain one directory at a time, the final cycle-closing directory remains visible but
   has no recursively repeated descendants. Project Explorer never hangs, grows the tree
   indefinitely, or loses the cycle boundary between lazy loads.

   Nested directory symlinks use the same rules at every depth:
   - A directory symlink discovered anywhere below an alias is a new lexical alias boundary. It is
     shown at the path reached through its parent alias and starts unloaded; classifying it does not
     enumerate its target.
   - Its target may be inside or outside the workspace, may also be reachable by another alias, and
     may contain further directory symlinks. Each lexical alias keeps independent expansion,
     selection, refresh, and destructive-action identity.
   - Its canonical target is checked against the full canonical ancestry inherited from the outer
     alias. A repeated identity produces one visible, loaded, childless cycle closer. A distinct
     identity extends the ancestry for later lazy expansions.
   - Retargeting or breaking a nested alias clears only that alias's stale subtree and watches; it
     does not collapse or replace readable siblings or the containing alias. An in-flight result
     from the old nested target cannot repopulate it.
   - Renaming or deleting a nested alias root changes or unlinks only that symlink entry. The target
     and its contents are never renamed or recursively deleted. Ordinary directories and existing
     file symlinks encountered below an alias retain their existing behavior.

9. Link failures follow this observable matrix and never block readable siblings:
   - If resolving the target reports `NotFound`, or the link is broken or no longer targets a
     directory, Warp removes the alias and all previously loaded descendants from Project Explorer.
   - If the alias root exists but reading it reports `PermissionDenied` or another transient I/O
     error, Warp keeps the alias as an unloaded directory with no stale children. Expanding it again
     retries the read.
   - If one descendant is unreadable, Warp keeps that descendant as an unloaded directory, omits
     its stale children, and continues loading readable siblings.
   - If the link is retargeted, Warp removes every child from the old target before showing children
     from the new target. Events from the old target cannot repopulate the alias.

10. Restoring access to an unreadable alias allows a later expansion to repopulate it without
    reopening the workspace. Restoring a broken target requires a link-path filesystem event or a
    normal Project Explorer/workspace refresh; Warp does not keep an unbounded watch on a missing
    external target.

11. Workspace ignore rules are evaluated against the workspace-visible symlink path. A rule that
    ignores the link or one of its aliased descendants has the same visible/lazy behavior as the
    equivalent rule for an ordinary directory.

12. A `.gitignore` inside the linked target applies to descendants viewed through that link. Ignore
    status does not leak between separate aliases except where the same workspace rule matches both
    alias paths.

13. Existing Project Explorer settings, including hidden-file visibility, apply to linked
    descendants exactly as they do to ordinary descendants.

14. Creating, modifying, renaming, moving, or deleting content in the target refreshes every
    expanded alias that points to the affected target. The same change also refreshes an ordinary
    workspace path to that target when one is present. Warp non-recursively watches each resolved
    directory that is actually expanded in Project Explorer. A child directory receives its own
    watch only when the user expands that child. Collapsing the last view of a directory releases
    that watch, and expanding it again refreshes the directory before presenting it as current.

15. Replacing or retargeting the symbolic link replaces its visible descendants with those of the
    new target. Events from the old target no longer update the alias after the retarget is
    observed.

16. Watcher-driven refreshes preserve alias identity: updates remain addressed by the
    workspace-visible path, do not introduce a second canonical-target path beneath the root, and
    do not move selection to another alias.

17. Lazy-loading depth, ignored-directory treatment, and non-ignored file budgets apply
    independently to each explicit expansion. Every expansion receives a fresh file budget.
    Exhausting that budget leaves remaining linked directories unloaded and available for a later
    expansion with a new budget rather than omitting unrelated workspace entries.

18. Existing symbolic links to files continue to appear and open as they do before this change.
    Existing project-skill discovery through symlinked provider directories also continues to find
    the same skill files through its existing explicit provider allowlist and `SKILL.md` discovery
    rule, independently of Project Explorer expansion. A nested Project Explorer alias does not
    become a skill provider merely because it is browsable; if its lexical path separately matches
    the configured skill rule, the existing skill-discovery path remains the sole owner of that
    standing-query result.

19. A malformed or inaccessible symlink anywhere below a linked target is isolated to that entry.
    Readable siblings and the rest of the repository continue to load and update.

20. Linked descendants added for Project Explorer are not repository contents. They do not appear
    in repository-wide search or indexing, automatically collected or ambient agent context,
    generic standing-query or skill-rule discovery, or serialized local-to-remote repository
    trees. The explicit **Attach as context** action in (4) may insert only the selected lexical path
    into the active terminal or agent input; it does not place overlay contents in the shared
    repository context. Only the existing explicitly configured symlinked-skill discovery behavior
    in (18) may otherwise read such a target outside Project Explorer.
