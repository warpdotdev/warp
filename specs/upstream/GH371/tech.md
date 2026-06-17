# TECH.md — Restore Open Files and Markdown Editors

**GitHub Issue:** [warpdotdev/warp-external#371](https://github.com/warpdotdev/warp-external/issues/371)
**Product Spec:** `specs/GH371/product.md`

## Problem

Code editor panes were persisted to SQLite on quit but restoration was disabled — the `restore_pane_leaf` function returned `Err("Can't restore code panes")` for `LeafContents::Code`, causing the code pane to be silently dropped from the restored pane tree.

Additionally, the original snapshot only captured a single file path (the active tab's path), but `CodeView` supports multiple open tabs (`tab_group: Vec<TabData>`). Multi-tab state was lost.

Markdown file panes (`FileNotebookView` via `NotebookPaneSnapshot::LocalFileNotebook`) are already correctly persisted and restored — no changes were needed for basic markdown restoration.

## Relevant Code

- `app/src/pane_group/mod.rs (1676-1679)` — `restore_pane_leaf`: the `LeafContents::Code` match arm that returns an error instead of restoring
- `app/src/pane_group/pane/code_pane.rs:210-212` — `CodePane::snapshot()`: serializes only the active tab's path
- `app/src/pane_group/pane/code_pane.rs:38-45` — `CodePane::new()`: constructor that takes a `CodeSource` and optional line/column
- `app/src/app_state.rs:197-210` — `CodePaneTabSnapshot` struct and `CodePaneSnapShot::Local` enum with `tabs`, `active_tab_index`, and `source`
- `app/src/persistence/sqlite.rs (1124-1133)` — `save_pane_state`: writes the code pane to SQLite
- `app/src/persistence/sqlite.rs (2423-2431)` — reads code pane from SQLite on startup
- `crates/persistence/src/schema.rs (115-121)` — `code_panes` table schema
- `crates/persistence/src/model.rs (601-606)` — `NewCodePane` insert model
- `crates/persistence/src/model.rs (430-437)` — `CodePane` queryable model
- `app/src/code/view.rs:230-239` — `CodeView` struct with `tab_group` and `active_tab_index`
- `app/src/code/editor_management.rs:103-131` — `CodeSource` enum
- `app/src/pane_group/pane/file_pane.rs:152-155` — `FilePane::snapshot()`: already works

## Current State

Persistence and restoration of code panes are both implemented. `CodePaneSnapShot::Local` stores all open tabs, the active tab index, and the `CodeSource`:

```rust
pub struct CodePaneTabSnapshot {
    pub path: Option<PathBuf>,
}

pub enum CodePaneSnapShot {
    Local {
        tabs: Vec<CodePaneTabSnapshot>,
        active_tab_index: usize,
        source: Option<CodeSource>,
    },
}
```

`restore_pane_leaf` destructures this single variant to reconstruct the `CodePane` via `CodeView::restore()`, which reopens each tab and selects the active one.

Markdown file panes (`FilePane`/`FileNotebookView`) continue to work via `NotebookPaneSnapshot::LocalFileNotebook`.

## Proposed Changes

### Implementation

Code pane restoration and multi-tab persistence were implemented together.

#### Snapshot model

`CodePaneSnapShot` has a single `Local` variant that holds all tabs, the active tab index, and the full `CodeSource`:

```rust
pub struct CodePaneTabSnapshot {
    pub path: Option<PathBuf>,
}

pub enum CodePaneSnapShot {
    Local {
        tabs: Vec<CodePaneTabSnapshot>,
        active_tab_index: usize,
        source: Option<CodeSource>,
    },
}
```

The `source` field stores the full `CodeSource` enum (serialized as JSON in SQLite) so that restored panes retain the correct source semantics. Variants whose extra data cannot be reconstructed at restore time fall back to `CodeSource::FileTree` if a path is available.

#### Snapshot creation

`CodePane::snapshot()` iterates the tab group, skipping preview tabs, and stores the source:

```rust
LeafContents::Code(CodePaneSnapShot::Local {
    tabs,
    active_tab_index,
    source,
})
```

#### SQLite persistence

Tabs are stored in a dedicated `code_pane_tabs` table (one row per tab, ordered by `tab_index`). The legacy `local_path` column on `code_panes` is still written for backward compatibility. The `CodeSource` is serialized as JSON into a `source` column on `code_panes`.

#### Restoration

In `restore_pane_leaf`, the `LeafContents::Code` arm (gated behind `#[cfg(feature = "local_fs")]`) destructures the snapshot and calls `CodeView::restore(&tabs, active_tab_index, source, ctx)` to rebuild the pane with all tabs.

### Phase 3: Markdown display mode preservation (optional enhancement)

The `FileNotebookView` display mode (rendered vs. raw) is not currently persisted. To support this:

1. Add a `display_mode` field to `NotebookPaneSnapshot::LocalFileNotebook`.
2. Persist via an optional column in `notebook_panes` (or encoded in `local_path` metadata).
3. On restore, pass the display mode to `FileNotebookView`.

This is lower priority since the rendered mode is the default and switching modes on restore is a single click.

## End-to-End Flow

### Quit (persist)

```
User quits → Workspace::snapshot_app_state()
  → PaneGroup::snapshot() walks the pane tree
    → CodePane::snapshot() → CodePaneSnapShot::Local { tabs, active_tab_index, source }
  → save_app_state() writes to SQLite
    → code_panes row + code_pane_tabs rows
```

### Launch (restore)

```
App starts → read_sqlite_data()
  → reads code_panes + code_pane_tabs → CodePaneSnapShot::Local
  → restore_pane_leaf()
    → CodeView::restore(&tabs, active_tab_index, source, ctx)
  → pane inserted into pane tree at correct position
```

## Risks and Mitigations

1. **Stale file paths.** A persisted path may no longer exist. The `CodeView`/`LocalCodeEditorView` already handles this gracefully (shows file-not-found state). The pane is created regardless. No extra handling is needed.

2. **CodeManager deduplication.** `CodePane::pre_attach()` checks `CodeManager` for duplicate paths in the same tab. During restoration, panes are created and attached in tree order, so the first pane to attach with a given path "wins". Subsequent panes with the same path would be deduplicated to the first. This matches existing runtime behavior and is acceptable.

3. **Migration safety.** The new DB columns are nullable, so the migration is backward-compatible. Existing databases with only `local_path` continue to work.

4. **Preview tabs and `pre_attach`.** Restored preview tabs may trigger `pre_attach` deduplication if the file is already open elsewhere. This is the same behavior as at runtime and acceptable.

## Testing and Validation

1. **Unit test for snapshot round-trip.** Test that `CodePane::snapshot()` produces a `CodePaneSnapShot` that, when written to and read from SQLite, matches the original data (single tab and multi-tab).

2. **Pane tree integrity test.** Verify that restoring a pane tree containing a mix of terminal, code, and notebook panes produces the correct tree structure with all panes present.

3. **Manual validation.** Follow the manual test steps in the product spec.

## Follow-ups

- **Scroll position and cursor restoration.** Could be added to `CodePaneTabSnapshot` in the future.
- **Unsaved buffer content.** Persisting dirty buffers is significantly more complex and out of scope.
- **Markdown display mode.** Phase 3 is optional and can be a separate PR.
