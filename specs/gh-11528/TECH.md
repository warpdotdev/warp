# Project Explorer Directory Symlinks — Technical Spec

## Context

This spec translates the local macOS Project Explorer-only contract in
[PRODUCT.md](./PRODUCT.md) into a bounded filesystem design. Remote-backed, Windows, and Linux
Project Explorer behavior is unchanged. The spec is grounded in Warp commit
[`abea51cd1e102b363935f1b25ef03d335bc7b36f`](https://github.com/warpdotdev/warp/tree/abea51cd1e102b363935f1b25ef03d335bc7b36f).

The current shared tree builder excludes directory symlinks before they become entries:

- [`Entry::build_tree_with_force_included_paths_and_ancestor`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/repo_metadata/src/entry.rs#L238-L430)
  skips a child when `is_symlink()` and `is_dir()` are both true.
- [`evaluate_entry`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/repo_metadata/src/entry.rs#L567-L635)
  rejects a directory symlink presented as a build root.

Project Explorer does not use `Entry::load` as its primary expansion path. The real UI flow is:

1. [`FileTreeView::load_directory_from_model`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/code/file_tree/view.rs#L1397-L1443)
   calls `RepoMetadataModel::load_directory` after the user expands an unloaded item.
2. [`RepoMetadataModel::load_directory`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/repo_metadata/src/wrapper_model.rs#L343-L372)
   delegates to the local model; tests use its completion-returning sibling directly.
3. [`LocalRepoMetadataModel::load_directory_with_completion`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/repo_metadata/src/local_model.rs#L1223-L1365)
   performs the background build, validates that the target did not change, applies the subtree,
   starts any required watch, and emits `FileTreeEntryUpdated`.

That model owns a shared repository tree. Project Explorer reads it directly, while repository
content APIs can feed indexing, search, and agent context. Putting external target descendants in
the shared tree would broaden #11528 beyond user-initiated browsing. The implementation therefore
needs a Project Explorer projection rather than changing the contents of the shared tree.

Warp already has a separate, explicit exception for symlinked project-skill providers:
[`add_symlink_target_updates` and `refresh_symlink_targets`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/repo_metadata/src/local_model.rs#L453-L532)
maintain standing-query results without materializing those directories in the canonical tree, and
[`entry_tests.rs`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/repo_metadata/src/entry_tests.rs#L424-L470)
locks in that behavior. Project Explorer traversal must remain independent of that allowlisted skill
rule.

## Proposed changes

### 1. Add a Project Explorer-only alias projection

Keep `IndexedRepoState::Indexed(FileTreeState)` as the shared, canonical repository tree. Add a
crate-private `ProjectExplorerAliasState` per local repository with:

- An overlay `FileTreeEntry` containing only lexical directory aliases and descendants loaded by
  Project Explorer.
- A map of `AliasTraversalCursor` values for unloaded overlay directories.
- A monotonically increasing generation for each alias root.
- Expansion/watch leases, keyed by Project Explorer subscriber and each expanded lexical overlay
  directory.

Return lightweight `ProjectExplorerAliasRoot` descriptors as side metadata whenever a shared build
encounters directory symlinks, while continuing to omit them from the shared `Entry`, flat `files`
list, standing-query results, and incremental remote updates. Discovery first uses
`symlink_metadata` to identify the lexical link, then follows metadata only far enough to classify
the target; it does not call `read_dir`. A target classified as a directory therefore receives an
unloaded descriptor even when enumerating its contents would return `PermissionDenied`.

The failure matrix's root read errors refer to `read_dir` after this type-only probe has already
identified a directory and created its descriptor. If the type probe itself cannot determine
whether a newly encountered symlink targets a directory, Project Explorer must not fabricate a
directory entry and change file-symlink behavior. A refresh or link-path event retries
classification; for an already-known alias, a transient type-probe failure preserves its unloaded
descriptor while clearing stale children. Descriptor production must run through all three
discovery lifecycles:

1. The initial repository build records aliases whose shared parents are loaded.
2. An ordinary shared lazy-parent expansion returns descriptors discovered immediately below the
   newly loaded parent and merges them into the overlay in the same completion callback.
3. Shared watcher create/remove/retarget deltas add, remove, or replace descriptors and overlay
   subtrees without waiting for a full repository rebuild.

Descriptors contain the lexical path and resolved target. Discovery does not traverse or watch the
target. If an alias sits below an unloaded shared parent, it becomes visible only after that ordinary
parent is expanded and its descriptor is returned.

Expose an explicit wrapper query such as `project_explorer_entry(repository_id)` that returns the
shared entry plus the overlay as a copy-on-write projection. Update only `FileTreeView` to use that
query when constructing or refreshing its visible root. `get_repository`, `get_repo_contents`, and
`standing_query_results` continue to expose shared state with no overlay. A distinct query/type
boundary is preferable to a visibility flag that every future consumer could forget to filter.

The overlay stores lexical paths, so Project Explorer selection, path copy, open, rename, and delete
actions keep using `/workspace/alias/...`. Canonical targets remain private model metadata and never
become `FileTreeEntry` keys.

### 2. Route UI expansion through the existing completion path

Keep `FileTreeView::load_directory_from_model` → `RepoMetadataModel::load_directory` →
`LocalRepoMetadataModel::load_directory_with_completion` as the authoritative UI path. At the local
model boundary:

- If `dir_path` is in the shared tree, preserve the existing load behavior.
- If `dir_path` has an `AliasTraversalCursor`, invoke a new scoped
  `build_project_explorer_alias_level` helper and apply its result to the overlay only.
- Validate the alias generation in the completion callback, just as the current code validates the
  unloaded entry identity, so an old async load cannot overwrite a retargeted link.
- Emit the existing `FileTreeEntryUpdated` event after an overlay update; the view then refreshes
  from `project_explorer_entry`.

`Entry::load` can remain unchanged and is not relied upon for this feature.

### 3. Persist traversal lineage across lazy expansions

Each `AliasTraversalCursor` contains:

- `alias_root` and `generation`.
- `lexical_path`, used for visible nodes, ignore matching, and file operations.
- `resolved_path`, used for `read_dir` and target watch translation.
- `canonical_lineage`, the canonical directory identities already visited on the route from the
  alias root through this unloaded directory.
- The inherited ignore state needed by the next load.

The invariant is `canonical_lineage.last() == resolved_path`: every cursor's lineage includes its
own resolved directory. Initialize an alias-root cursor with
`canonical_lineage = [canonical_target]`. A child cursor clones the parent lineage and appends the
child's resolved identity only after confirming it is not already present.

When one level is loaded, form a child lexical path from the discovered filename without
canonicalizing that visible path. Resolve the filesystem identity separately. Before emitting an
unloaded child cursor, compare its resolved identity with the persisted lineage:

- If it repeats an ancestor, emit a loaded, childless lexical directory and no cursor.
- Otherwise append the child's resolved directory to the lineage and persist the resulting cursor
  with the child.

Persist cursors for ordinary and symlinked directories below an alias, not only for the alias root.
This is what preserves the boundary when the user expands `A`, then `B`, then `A` in separate model
tasks. A process-local ancestry set inside one tree walk is insufficient.

### 4. Keep traversal out of indexing, context, standing queries, and remote sync

Make traversal purpose explicit with a private enum or separate APIs:

- `SharedRepository` builds the current canonical tree, files/index inputs, and standing-query
  results. It may collect unloaded alias-root descriptors but never their contents.
- `ProjectExplorerAlias` builds only overlay nodes and cursors. It cannot receive a shared `files`
  sink or `StandingQueryResults`, and its result type cannot be passed to
  `flatten_entry_metadata`.
- Existing explicit symlinked project-skill discovery remains on its current standing-query path
  and allowlisted provider rules; Project Explorer expansion neither adds nor removes those results.

Enforce the boundary at these points:

1. `LocalRepoMetadataModel::get_repo_contents` traverses only `FileTreeState.entry`, so repository
   search/indexing and agent-context callers cannot observe overlay descendants.
2. `standing_results` are updated only by shared/explicit standing-query builds. Alias loads do not
   call `record_path`, `record_followed_project_skill_directory`, or skill-rule matchers.
3. `FileTreeMutation`, `RepoMetadataUpdate`, and `flatten_entry_metadata` accept only shared-tree
   results. Overlay mutations emit a local view refresh but never `IncrementalUpdateReady`.
4. Remote snapshot and incremental serialization omit alias roots, cursors, target paths, and loaded
   overlay contents. Remote-backed, Windows, and Linux Project Explorer behavior is unchanged by
   this local macOS fix.
5. Repository removal destroys both the shared state and its local-only overlay/leases; no overlay
   state is persisted to disk.

This preserves PRODUCT invariants 18 and 20 and prevents an external link from silently enlarging
the codebase indexed or supplied to an agent.

### 5. Define classified failure handling

Classify alias-resolution and read failures before applying the completion result:

| Condition | Overlay result | Cursor/watch result |
| --- | --- | --- |
| `NotFound`, broken link, or target no longer a directory | Remove alias root and stale descendants | Drop all cursors, generation, and leases for the alias |
| Root `read_dir` returns `PermissionDenied` after directory classification | Replace stale subtree with the unloaded alias root, including on first expansion | Retain a retry cursor; release the target watch if it cannot be maintained |
| Other transient root `read_dir` error after directory classification | Replace stale subtree with the unloaded alias root, including on first expansion | Retain a retry cursor; keep only a valid existing expansion lease |
| Unreadable descendant | Keep that descendant as an unloaded placeholder; continue siblings | Persist a retry cursor for that descendant |
| Retargeted link | Remove old subtree before installing a new unloaded alias root | Increment generation, release old-target lease, discard stale completions |

The completion future may still resolve with a typed `RepoMetadataError` for logging/tests, but its
callback first establishes the table's safe visible state and emits a refresh. Error display and
logging must not include the canonical external target path: structured events record only the
workspace-relative lexical alias path, alias generation, and error class, and user-visible errors
follow the same rule. Tests may inspect typed fields directly but must not format or snapshot an
external target path. A retry always starts from the retained cursor and revalidates the current
link target.

### 6. Bound target watches to expanded aliases

Initial lazy alias discovery never registers an external-target watch. Add a
`ProjectExplorerAliasLease` owned by each active `FileTreeView` subscription:

- Expanding an alias directory acquires a lease for that directory's `resolved_path` only after
  resolution succeeds. If the directory is already covered by the repository root watcher, no extra
  watch is registered.
- An external resolved directory uses `RecursiveMode::NonRecursive`, reference-counted by canonical
  directory across views and aliases. Expanding a child directory acquires a separate non-recursive
  watch for that child's resolved path; an unexpanded child never causes a watch.
- Events from a leased directory are translated only to the direct lexical children of currently
  leased aliases for that resolved directory. Deeper updates are observed by the separate lease for
  the expanded descendant that contains them.
- Collapsing a directory (including any expanded descendants hidden by that collapse),
  deactivating/dropping its view, removing the repository, breaking the link, or retargeting it
  releases the corresponding leases. Each external watch is unregistered when its count reaches
  zero.
- A collapsed overlay may remain cached, but it is treated as stale. Re-expansion revalidates and
  refreshes the alias before presenting it as current and reacquires the lease.

Wire both branches of `FileTreeView::toggle_folder_expansion` to a model method such as
`set_project_explorer_alias_expanded(subscriber, repo, path, expanded)`. This notification must run
even when the cached directory state is already `loaded`; otherwise re-expansion would skip refresh
and lease acquisition because the existing `ensure_loaded_path` early return sees a loaded entry.

Target events translate the direct-child suffix below the watched resolved directory onto every
leased lexical directory. If a precise event cannot be translated safely, rebuild that one lexical
directory level. Never send these events through shared-tree mutations or remote incremental
serialization.

### 7. Preserve ignore, visibility, and budget semantics

Evaluate repository ignore patterns against the lexical alias path. Reading
`lexical_path/.gitignore` follows the filesystem link, but matches remain relative to that alias.
Hidden-file filtering stays in `FileTreeView`, so the merged projection follows the existing
`show_hidden_files` setting without a new rendering path.

Initialize a fresh `LAZY_LOAD_FILE_LIMIT` for every explicit alias-directory expansion, matching the
existing local-model load path. Do not persist a remaining budget in `AliasTraversalCursor`.
Directory placeholders and cycle closers do not consume file quota. If an expansion exhausts its
fresh quota, emit the remaining directories as unloaded entries with cursors; expanding one later
starts with another fresh `LAZY_LOAD_FILE_LIMIT`. Alias loads never spend the shared repository's
indexing budget.

No feature flag is proposed: the change is a scoped correction to local macOS Project Explorer
behavior, introduces no setting, and is compiled behind the existing `local_fs` plus macOS target
boundary. Non-macOS and remote wrapper paths continue to return their existing trees.

## Testing and validation

### Automated tests

Use macOS-gated real temporary directories and `std::os::unix::fs::symlink` for filesystem
behavior. Add:

1. `view_tests.rs`/wrapper coverage proving a user expansion follows
   `FileTreeView` → `RepoMetadataModel::load_directory` →
   `LocalRepoMetadataModel::load_directory_with_completion`, then refreshes the projected entry.
2. Traversal tests proving a readable alias starts unloaded, stores lexical children, and supports
   both in-workspace and external targets without inserting canonical target paths (PRODUCT 1–6).
3. Descriptor-lifecycle tests covering an alias found during initial indexing, an alias below an
   initially unloaded ordinary shared parent discovered only when that parent loads, and alias
   create/remove/retarget watcher deltas after indexing. Include a directory that is classified and
   added without enumerating its unreadable contents, then restores access and populates on retry.
   Assert every path adds or removes the same unloaded overlay descriptor without traversing or
   watching its target.
4. A multi-step completion test that expands `A → B → A` in three separate
   `load_directory_with_completion` calls and asserts the final `A` is loaded and childless. Also
   assert every cursor includes its own `resolved_path` as the last lineage item; cover root
   initialization, a direct self-cycle, and two independent aliases to one target (PRODUCT 7–8).
5. Table-driven local-model tests for every failure-matrix row: `NotFound`, broken/non-directory,
   root `read_dir` `PermissionDenied`, other transient root read errors, unreadable descendant with
   readable sibling, a transient type-probe failure for an already-known alias, and retarget during
   an in-flight load. Assert initial enumeration failures remain visible and retryable, and stale
   children/cursors/watches are removed exactly as specified (PRODUCT 9–10, 15, 19). Add a
   logging/redaction assertion proving a typed external-target error formats only the lexical alias
   path, generation, and error class.
6. Scope-boundary tests proving an expanded external alias is present in
   `project_explorer_entry` but absent from `get_repo_contents`, repository search/index input,
   agent-context collection, generic standing-query/skill-rule results, `RepoMetadataUpdate`, and
   remote snapshots. Retain the existing explicit symlinked project-skill discovery regression
   unchanged (PRODUCT 18, 20).
7. Watch-lease tests proving initial lazy aliases create zero external watches; expanding the alias
   root creates one non-recursive watch; an unexpanded child creates none; expanding the child adds
   its own non-recursive watch; a second alias/view shares matching resolved-directory watches; and
   collapse, retarget, repository removal, and view teardown release the exact leases. Re-expansion
   must refresh before display (PRODUCT 14–16).
8. Project Explorer action tests selecting and opening a descendant and invoking copy, rename, and
   delete paths, asserting every emitted/requested path uses the lexical alias rather than the
   canonical target (PRODUCT 4).
9. `view_tests.rs` coverage with hidden files and directories below an alias, verifying the existing
   setting hides them by default and reveals them after `show_hidden_files` changes (PRODUCT 13).
10. Budget/depth tests proving every explicit expansion starts with a fresh
    `LAZY_LOAD_FILE_LIMIT`, cursors retain no remaining budget, cycle placeholders spend no quota,
    and budget-exhausted child directories remain unloaded for a later fresh-budget expansion
    without changing the shared repository (PRODUCT 17).
11. Ignore tests proving workspace rules match lexical paths and a target `.gitignore` applies only
    within that alias projection (PRODUCT 11–12).
12. File-symlink regression tests and overlay generation tests proving stale async completion cannot
    overwrite a retargeted alias (PRODUCT 15, 18).

Implementation PR checks:

```sh
cargo nextest run -p repo_metadata --features local_fs
cargo nextest run -p warp --features integration_tests -E 'test(file_tree)'
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```

Both `./script/format` and the full workspace clippy command must pass before opening or updating the
PR, as required by `AGENTS.md`. Cross-platform CI must compile the unchanged non-macOS paths even
though symlink-creation behavior tests are macOS-gated.

### Manual evidence

On macOS, create one workspace-local target and one external target, then capture:

1. Before/after Project Explorer screenshots showing each alias expanded under its lexical path.
2. A short recording that opens a target file through its alias, changes target content while
   expanded, collapses/re-expands the alias, and retargets the link without stale children.
3. A cycle fixture showing repeated `A → B → A` expansion terminates and Warp remains responsive.

The issue screenshots establish the before state. No Linux-only visual evidence is required for
this macOS-reported bug.

## Parallelization

After agreeing on `ProjectExplorerAliasState`, `AliasTraversalCursor`, and the projection API, two
local agents can work in parallel and land in one implementation PR:

- **Traversal/projection agent** — owns the alias traversal module, projection merge, and their unit
  tests; worktree `../warp-gh11528-alias-tree`, branch `codex/gh11528-alias-tree`. It implements
  lexical/resolved paths, persisted lineage, error classification, ignore handling, and quotas.
- **UI/watcher agent** — owns `wrapper_model.rs`, `local_model.rs`, `view.rs`, and corresponding
  model/view tests; worktree `../warp-gh11528-alias-ui`, branch `codex/gh11528-alias-ui`. It wires
  the real completion path, projection refresh, leases, event translation, and teardown.

An integration owner uses `../warp-gh11528-integration` on
`codex/gh11528-symlink-directories`, merges traversal/projection before UI/watcher, adds the
cross-surface scope-boundary tests, runs validation, and opens one implementation PR. Agents do not
edit sibling-owned files and must agree on the cursor/result interfaces before starting. Manual
macOS evidence runs only after integration.

## Risks and mitigations

1. **Alias contents could leak into indexing or agent context.** Keep the overlay behind an explicit
   Project Explorer projection type/API and add negative tests at every shared export boundary.
2. **Lazy loads could forget cycle ancestry.** Persist canonical lineage in every unloaded alias
   cursor and test `A → B → A` across separate completion tasks.
3. **Stale loads could repopulate a retargeted link.** Tie cursors and completion callbacks to an
   alias generation and discard mismatches.
4. **External watches could grow without bound.** Acquire them only for expanded aliases, share them
   by canonical target, and release them on collapse, view/repository teardown, breakage, or
   retargeting.
5. **Permission failures could leave stale or misleading children.** Apply the classified failure
   matrix before completing the task and retain retry cursors only where the target may recover.
6. **Canonicalization could erase visible identity.** Store canonical paths only in cursors/watch
   leases; assert every overlay node and UI operation uses the lexical prefix.
7. **Explicit skill discovery could regress.** Leave its current standing-query path separate and
   retain its existing symlink regression test unchanged.
