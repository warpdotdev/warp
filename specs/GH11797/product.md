# PRODUCT.md — Jujutsu (jj) prompt chips

**GitHub Issue:** [warpdotdev/warp#11797](https://github.com/warpdotdev/warp/issues/11797)

## Summary

Add two new prompt context chips for the `jj` (Jujutsu) VCS: `JjBookmark` (bookmark/change display) and `JjDirtyItems` (changed-file count), mirroring the existing `SvnBranch` / `SvnDirtyItems` chip pattern. Users can add these chips to their prompt via the existing chip picker UI.

## Problem

Warp currently supports prompt chips for Git and SVN but has no equivalent for `jj`. Users who rely on `jj` instead of Git lack VCS-aware prompt chips showing their current bookmark/change context and workspace dirtiness.

## Goals

1. Add a `JjBookmark` chip that displays the current `jj` bookmark or abbreviated change ID, wrapped as `jj:(...)` in the prompt.
2. Add a `JjDirtyItems` chip that displays the number of changed files in the `jj` workspace.
3. Mirror the SVN chip architecture (`builtins.rs` shell commands + `mod.rs` enum dispatch) so the implementation is minimal and consistent.
4. Both chips degrade gracefully when `jj` is not installed or the working directory is outside a `jj` repository.

## Non-goals

- Full `jj` VCS support in Warp (tracked in #11774).
- Bookmark-list menu on click — intentionally out of scope for the initial implementation.
- Configurable display format beyond the established `jj:(…)` / `±N` conventions.
- Automatically tracking `jj` bookmark changes — both chips refresh on demand only.

## Behavior

1. **Chip availability.** `JjBookmark` and `JjDirtyItems` appear in the chip picker UI (`available_chips()`) alongside other VCS chips. Users can add, remove, and reorder them freely.

2. **Runtime disable.** Both chips require the `jj` executable. When `jj` is absent from the session's `$PATH`, each chip is disabled with a tooltip: `Requires the \`jj\` command`.

3. **JjBookmark — bookmarked change.** When the current change has one or more bookmarks, the chip shows `jj:(<bookmarks>)`. Bookmarks are space-separated when multiple exist. Example: `jj:(main)`, `jj:(my-feature stable)`.

4. **JjBookmark — anonymous change, bookmarked ancestor exists.** When the current change is anonymous but an ancestor has bookmarks, the chip shows `jj:(<change_id_short8> on <bookmarks>)`. Example: `jj:(f3a2b1c0 on main)`.

5. **JjBookmark — anonymous change, no bookmarked ancestor.** When neither the current change nor any ancestor has bookmarks, the chip shows `jj:(<change_id_short8>)`. Example: `jj:(f3a2b1c0)`.

6. **JjBookmark — no `jj` repository.** When not inside a `jj` repository, the shell command produces no output and the chip is hidden (empty value, no rendered element).

7. **JjDirtyItems — workspace changes.** The chip shows the number of files with outstanding changes in the working copy, as reported by `jj diff --summary`. When there are no changes, the chip is hidden. Example: `3` (three files changed).

8. **Prompt rendering — JjBookmark.** The chip renders as `jj:(<value>)` in the prompt, using `input_prompt_branch` for the prefix `jj:(`, suffix `)`, and value text. The font weight is Semibold.

9. **Prompt rendering — JjDirtyItems.** The chip renders as `±<value>` using `input_prompt_svn` for the prefix and value color, mirroring `SvnDirtyItems`.

10. **Adjacent chip spacing.** When `JjBookmark` and `JjDirtyItems` are adjacent in the prompt (in either order), no space is inserted between them, matching the existing `SvnBranch`/`SvnDirtyItems` behavior.

11. **Placeholder values.** In the chip picker settings UI, the placeholder for `JjBookmark` is `jj-feature-bookmark` and for `JjDirtyItems` it is `3`.

12. **Icon.** Both chips reuse the existing Git/SVN icons: `Icon::GitBranch` for `JjBookmark` and `Icon::File` for `JjDirtyItems`.
