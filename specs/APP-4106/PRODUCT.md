# APP-4106: Group by shared root in file tree

Linear: https://linear.app/warpdotdev/issue/APP-4106/group-by-shared-root-in-file-tree

## Summary

The Project Explorer file tree currently shows every "working root" the workspace
tracks as a separate top-level entry. When one working path is an ancestor of
another (e.g., a file open at `~/code/foo.ts` and a terminal cwd at
`~/code/a`), this produces redundant, nested-looking roots. This feature makes
the tree collapse those cases into a single shared root and auto-expand the
absorbed descendant path so the user's current focus is still visible.

## Problem

Today's display logic deduplicates only by exact path equality
(`deduplicate_by_directory_name` in `left_panel.rs`). As a result:

- Opening a file outside any repo and having a terminal cwd under the same
  directory tree produces two separate roots, one of which visually contains
  the other.
- cd-ing deeper into an already-displayed directory produces a second root
  rather than navigating inside the existing one.
- Users see a cluttered multi-root tree that does not match how they think
  about their workspace.

Global Search already dedupes descendant roots via `deduplicate_search_roots`;
the file tree does not. That inconsistency is part of the bug.

## Goals

1. When one displayed root is a strict ancestor of another, collapse to a
   single root (the ancestor) and auto-expand the chain down to each absorbed
   descendant so the user can see where their focus is.
2. Keep unrelated siblings as distinct roots — do not synthesize a shared
   ancestor that is not already one of the active paths.
3. Preserve the user's explicit collapse state. If the user manually collapsed
   a folder, auto-expand should not silently re-open it.
4. Keep the tree's behavior consistent with Global Search's existing
   ancestor-dedup logic (same rule, one source of truth in code).

## Non-goals

- Computing a synthetic greatest common ancestor for unrelated sibling paths
  (e.g., `~/code/a` + `~/code/b` must NOT collapse to `~/code`).
- Changing how Warp detects git repositories or resolves terminal cwds to
  repo roots upstream in `WorkingDirectoriesModel`.
- Changing the remote-repo root insertion policy
  (`insert_or_update_remote_root`) — remote pushes continue to use their
  existing "new root wins" sweep.
- New UI affordances such as badges or breadcrumbs indicating where the
  user's cwd is inside a collapsed root.

## Figma / design references

Figma: none provided. This is a behavior change in an existing panel; no new
visual components are introduced.

## User experience

### Definitions

- "Active path" = a path reported by `WorkingDirectoriesModel` for the active
  pane group. These are terminal cwds (resolved to their repo root when one
  exists) and code-editor file-parent paths (resolved to their repo root when
  one exists).
- "Displayed root" = a top-level entry in the Project Explorer file tree.
- Ancestor comparison uses path-prefix semantics on the active path strings.
  `~/code` is an ancestor of `~/code/a`; `/a` is NOT an ancestor of `/ab`.

### Invariant 1: collapse descendants into their ancestor

If the set of active paths contains both path `A` and path `B` where `A` is a
strict ancestor of `B`, only `A` appears as a displayed root. `B` is treated
as "absorbed by `A`" and is no longer a top-level entry. This applies
transitively: with `[~/code, ~/code/a, ~/code/a/z]`, only `~/code` is
displayed.

### Invariant 2: unrelated siblings remain separate

If no active path is an ancestor of another, every active path is shown as its
own displayed root, in the same most-recent-first order as today. Examples
that stay as two roots:

- `~/code/a` + `~/code/b`
- `~/code/a` + `~/other`
- `/a` + `/ab`

Warp does not synthesize a new common root (like `~/code` or `/`) that is not
already an active path.

### Invariant 3: auto-expand the chain to each absorbed descendant

When a descendant is absorbed into an ancestor root, the tree expands each
intermediate directory on the path from the ancestor down to the absorbed
descendant so that the absorbed descendant's folder is visible in the tree.

Example: with active paths `[~/code, ~/code/a/z]`, the tree shows `~/code`
expanded, `a` expanded, and `z` expanded (contents visible).

Auto-expansion stops at the first directory that the user has explicitly
collapsed (see Invariant 4).

### Invariant 4: explicit collapse is respected

If the user has explicitly collapsed a directory (via the chevron or the
keyboard `Collapse` action), a later absorption event must NOT re-expand that
directory.

Example: user has `~/code` as a root with `~/code/a` explicitly collapsed.
User cds into `~/code/a/z`. The tree leaves `~/code/a` collapsed and does
not expand `z`. The displayed-roots invariant (Invariant 1) still holds:
only `~/code` is a displayed root, and the cwd change does not add `~/code/a`
or `~/code/a/z` as new roots.

### Invariant 5: focus-follow on cd into a descendant

When the user cds into a path that is a descendant of an existing displayed
root, the tree treats the new cwd as the "most recent" focus:

- Auto-expansion runs to reveal the new cwd (subject to Invariant 4).
- The cwd's directory header is selected in the tree.
- On the first apply for a given cd, the tree scrolls the cwd's directory
  header to the top of the viewport (not merely into visibility) so a
  fresh cd looks like the cwd took over the top of the tree. Subsequent
  rebuilds that re-apply the same focus (e.g. repo-metadata updates) must
  NOT re-scroll — see Invariant 9.

### Invariant 6: state migration when an existing root is absorbed

If a directory that was previously a top-level root gets absorbed into a new
ancestor root, the tree preserves the user's per-root state:

- Expanded folders under the absorbed root remain expanded under the
  surviving root.
- Explicitly collapsed paths under the absorbed root remain explicitly
  collapsed under the surviving root.
- If the user's selection lived inside the absorbed root, the selection
  remains on the same file/directory path after absorption. If that path is
  no longer reachable (e.g., because a parent got explicitly collapsed
  during migration), the selection clears.
- Item-level interaction state (mouse hover, drag state) is not guaranteed to
  persist through a root-shape change; losing a transient hover state is
  acceptable.

### Invariant 7: remote roots are unaffected

Remote-backed roots (pushed by `insert_or_update_remote_root`) continue to
use their existing ancestor/descendant sweep (new root wins). The new
local-root grouping does not run on remote roots and does not reorder them.

### Invariant 8: Global Search stays consistent

Global Search's existing behavior — "drop descendants when an ancestor is
present" — continues to apply, using the same underlying path-deduplication
helper as the file tree. Whatever the file tree displays as its set of
ancestor roots matches the set Global Search will search over for the same
active pane group.

### Invariant 9: user scrolling is respected

After the initial cd-follow scroll lands the cwd at the top of the
viewport, subsequent events that would have targeted the same cwd (e.g.
late repo-metadata updates, file-watcher rebuilds) must NOT re-scroll the
tree back to the cwd. The selection marker may be preserved across those
rebuilds, but scroll position is the user's to control once the initial
placement has happened.

### Invariant 10: explicit user focus wins over cwd-follow

If the user clicks a file in the tree (or the active code editor focuses a
file), that explicit selection must not be overridden by a cwd-follow
triggered as a side effect of the same action. Concretely: when a
`DirectoriesChanged` event fires because a code view just opened a file,
and the current selection is already at or under the path the cd-follow
would target, no cd-follow is recorded. The user's file-level selection is
more specific than the generic directory-level target.

### Invariant 11: new root takes focus on cd

When a cd introduces a brand-new top-level root (one that isn't an
ancestor or descendant of any existing displayed root), the file tree
moves selection to that new root's header. An existing selection under a
different (now non-most-recent) root is replaced. This matches today's
behavior for cd-ing into a fresh directory that isn't under any existing
root.

### Edge cases

- **Root and its own ancestor are both active**: covered by Invariant 1.
  Example: `[~, ~/code]` → `~` is the only displayed root; `~/code` is
  expanded inside it.
- **Three-deep chain**: `[~/code, ~/code/a, ~/code/a/z]` → one root
  (`~/code`), with `a` and `z` auto-expanded.
- **Mixed ancestor + unrelated sibling**: `[~/code, ~/code/a, ~/other]` →
  two roots (`~/code`, `~/other`); `~/code/a` absorbed and expanded.
- **Same prefix, different directory names**: `[/foo/a, /foo/abc]` → two
  roots (`/foo/a` is not an ancestor of `/foo/abc`).
- **Reverse-order input**: `[~/code/a, ~/code]` (descendant listed first) →
  one root (`~/code`); absorption is symmetric in input order.
- **Active path equals itself**: duplicate input paths dedupe as today (they
  don't trigger absorption of "self").
- **cd into a path outside any existing root**: behaves as today — a new
  displayed root is added. Grouping only collapses ancestor/descendant
  relationships among the active path set.
- **Explicit collapse on an ancestor chain link blocks further auto-expand**:
  if `~/code/a` is explicitly collapsed and the user cds into
  `~/code/a/z/inner`, the tree leaves `~/code/a` collapsed. `~/code` stays
  expanded, `~/code/a` stays collapsed, `~/code/a/z` and deeper are NOT
  auto-expanded because expansion halted at the collapsed link.

## Success criteria

1. Given active paths `[~/code/foo.ts-parent, ~/code/a, ~/code/a/z]`, the
   Project Explorer shows exactly one root `~/code`, with `a` and `z`
   expanded, and `~/code/a/z` selected and scrolled to the top of the
   viewport.
2. Given active paths `[~/code/a, ~/code/b]`, the Project Explorer shows two
   roots `~/code/a` and `~/code/b` in most-recent-first order, matching
   today's behavior.
3. cd-ing from `~/code` to `~/code/a/z` inside the same terminal does not
   add a second root; the existing `~/code` root auto-expands and selects
   `~/code/a/z`, and `~/code/a/z` is scrolled to the top.
4. A folder the user has explicitly collapsed stays collapsed across
   subsequent absorption events and cd navigations.
5. When a previously top-level displayed root is absorbed, the user's
   previous expansion and explicit-collapse state survives the migration
   (visible in the resulting tree shape).
6. The set of roots Global Search searches for the same pane group matches
   the set of roots the file tree displays (no divergence between the two
   panels).
7. Remote-backed roots continue to behave as they do today, including the
   existing "new root wins" sweep.
8. Tree identifiers remain valid after absorption — keyboard navigation,
   context menus, and drag/drop continue to work without errors after a
   root change.
9. After the initial cd-follow scroll, the user can scroll freely and
   subsequent repo-metadata-driven rebuilds (for the same cd target) do
   NOT snap the scroll position back.
10. Clicking a file in the tree leaves that file as the selected item;
    the `DirectoriesChanged` side effect of opening the file does not
    override the selection with the file's parent directory.
11. Cd-ing into a brand-new root moves selection to the new root's header
    and scrolls there.

## Validation

1. **Rust unit tests** for the shared `group_roots_by_common_ancestor`
   helper (`crates/warp_util`): verify each invariant 1–2 and every edge
   case above, using concrete path inputs and asserting the resulting
   surviving-root list and absorbed-descendant map.
2. **`FileTreeView` view tests** (`app/src/code/file_tree/view/view_tests.rs`,
   `VirtualFS::test` harness):
   - Assert displayed roots and expanded folders for each scenario from
     Success Criteria 1–5.
   - Assert selection remains on the same path after an absorption event.
   - Assert explicit-collapse state survives migration.
3. **Global Search parity test**: assert that `GlobalSearchView` and
   `FileTreeView` compute the same surviving-root set given the same input.
4. **Manual verification (Dogfood)** using `verify-ui-change-in-cloud`:
   - Open `~/code/foo.ts` in the code editor, then cd the terminal from
     `~/code` → `~/code/a` → `~/code/a/z`. Confirm the file tree shows one
     `~/code` root with the chain expanded, and `z` is selected and
     scrolled to the top of the viewport.
   - Collapse `~/code/a` explicitly, then cd deeper. Confirm the tree does
     not re-expand `~/code/a`.
   - Open two terminals, one at `~/code/a`, one at `~/code/b`. Confirm two
     separate roots.
   - Cd to trigger a cd-follow, then scroll the file tree manually.
     Trigger a repo-metadata rebuild (e.g. edit a file the watcher sees)
     and confirm the scroll position is preserved.
   - Click a file in the tree and confirm selection stays on the file,
     not the file's parent directory.

## Open questions

None. Resolved during spec review:

- No additional UX polish is required when the focused file lives inside
  an absorbed descendant; the existing `find_deepest_root_for_file`
  behavior is sufficient.
- Auto-expansion order across multiple absorbed descendants is not a
  product concern; implementation may choose any order.
