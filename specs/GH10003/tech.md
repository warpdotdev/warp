# Tech Spec: Reload File Tree action in Command Palette

**Issue:** [warpdotdev/warp#10003](https://github.com/warpdotdev/warp/issues/10003)

## Context

Warp's File Tree (Wildtree) lives in [`app/src/code/file_tree/`](https://github.com/warpdotdev/warp/blob/master/app/src/code/file_tree/). The tree is backed by `repo_metadata`'s `LocalRepoMetadataModel` (for Git-tracked workspaces) or by a direct directory scan (for non-Git workspaces). Filesystem watchers feed automatic-update events; the tree re-renders on those events.

The Command Palette already exposes reload-style actions for adjacent surfaces (e.g. settings reload). This spec adds one more entry to that surface that triggers a forced re-scan of the active workspace's tree.

### Relevant code

| Path | Role |
|---|---|
| `app/src/code/file_tree/snapshot.rs` and `app/src/code/file_tree/snapshot/` | The tree's in-memory representation. Has a `from_disk` (or equivalent) constructor that performs a full scan. |
| `app/src/code/file_tree/view.rs` | Tree rendering. Subscribes to model events to re-render. |
| `crates/repo_metadata/src/local_model.rs` | `LocalRepoMetadataModel` — the authoritative tree source for Git-tracked projects. The reload path invokes its existing rebuild entry point. |
| `app/src/search/command_palette/static_actions.rs` (or equivalent — verify at implementation time) | The Command Palette source that registers static actions like Settings, Reload, etc. The new `Reload File Tree` action is added here. |
| `app/src/server/telemetry/events.rs` | Telemetry. New `FileTreeReloadInvoked { duration_ms: u32 }` event. |
| `app/src/workspace/toast.rs` (or equivalent `ToastStack`) | Existing toast infrastructure for the success/failure notification. |

### Related closed PRs and issues

- The existing palette static-action registration pattern (used by `Reload Settings`, etc.) is the closest analog. The Tab Configs palette spec (#9176) covers a parallel data-source addition; this is simpler because it's a single static action, not a new data source.

## Crate boundaries

All new code lives in `app/`. No new crate, no cross-crate boundary changes. The reload triggers an existing public function on `LocalRepoMetadataModel` and an existing public function on the non-Git tree path; the new code orchestrates them rather than implementing reloading from scratch.

## Proposed changes

### 1. New static palette action

**File:** `app/src/search/command_palette/static_actions.rs` (or wherever existing actions like `Reload Settings` are registered — verify at implementation time).

Pseudocode:

```rust
pub const RELOAD_FILE_TREE: StaticPaletteAction = StaticPaletteAction {
    label: "Reload File Tree",
    aliases: &["Refresh File Tree", "Reload Wildtree", "Refresh Wildtree"],
    icon_path: "bundled/svg/refresh.svg",
    availability: Availability::WORKSPACE_OPEN,
    action: PaletteActionId::ReloadFileTree,
};
```

The exact `StaticPaletteAction` type might be different in master (verify); the conceptual fields are: a primary label, alias strings indexed for fuzzy search, an icon, an availability gate, and a dispatch ID.

### 2. Availability gate

**File:** the existing `Availability` flags definition (already used by static slash commands per `app/src/search/slash_command_menu/static_commands/mod.rs`).

`WORKSPACE_OPEN` already exists for similar surfaces. If the action is a Command Palette action rather than a slash command, the equivalent gate is the palette's existing per-action availability check — extend it to a "tree exists" check that the existing tree-aware actions already use.

### 3. Dispatch arm

**File:** the palette dispatch site (locate via `grep -rn "PaletteActionId::" app/src/search/command_palette/`).

```rust
PaletteActionId::ReloadFileTree => {
    let started = Instant::now();
    match reload_active_file_tree(ctx) {
        Ok(()) => {
            let elapsed = started.elapsed().as_millis() as u32;
            ctx.show_toast(format!(
                "File tree reloaded ({} ms).",
                elapsed,
            ));
            send_telemetry_from_ctx(
                ctx,
                TelemetryEvent::FileTreeReloadInvoked { duration_ms: elapsed },
            );
        }
        Err(err) => {
            ctx.show_toast(format!(
                "File tree reload failed: {err}",
            ));
            // Telemetry still emits, with a flag — useful for measuring failure rate.
            send_telemetry_from_ctx(
                ctx,
                TelemetryEvent::FileTreeReloadInvoked {
                    duration_ms: started.elapsed().as_millis() as u32,
                },
            );
        }
    }
}
```

### 4. Reload entry point

**File:** new `app/src/code/file_tree/reload.rs`.

```rust
/// Force a full re-scan of the active workspace's File Tree.
/// Returns when the tree's in-memory model has been rebuilt from disk and
/// any pending render-side updates have been queued.
pub fn reload_active_file_tree(ctx: &AppContext) -> anyhow::Result<()> {
    let active_workspace = active_workspace_root(ctx)
        .ok_or_else(|| anyhow::anyhow!("no active workspace"))?;

    // Two paths depending on whether the workspace is Git-tracked.
    let is_git = repo_metadata::repositories::DetectedRepositories::as_ref(ctx)
        .get_root_for_path(&active_workspace)
        .is_some();

    if is_git {
        // Reuse the existing repo-rebuild path. This is the same code that
        // runs when a fresh DetectedGitRepo event fires; calling it directly
        // is the supported "force rebuild" surface.
        repo_metadata::local_model::LocalRepoMetadataModel::handle(ctx)
            .update(ctx, |model, ctx| {
                model.rebuild_repository(&active_workspace, ctx)
            })?;
    } else {
        // Non-Git tree path: directly reconstruct the snapshot from disk and
        // notify the view. The non-Git tree is a thinner abstraction —
        // identify the existing snapshot owner via grep and call its rebuild.
        crate::code::file_tree::snapshot::SnapshotOwner::handle(ctx)
            .update(ctx, |owner, ctx| owner.rebuild_from_disk(&active_workspace, ctx))?;
    }

    Ok(())
}

fn active_workspace_root(ctx: &AppContext) -> Option<PathBuf> {
    // The active session's CWD or workspace root. Implementation reuses the
    // existing helper that the File Tree itself uses to know what to render —
    // verify the exact entry point at implementation time.
    crate::workspace::active_workspace_root(ctx)
}
```

`rebuild_repository` and `rebuild_from_disk` are conceptual placeholders; the actual function names need to be confirmed at implementation time. The key contract: both paths exist already (they handle the automatic-refresh case); reload simply invokes them on user demand.

### 5. Telemetry

**File:** `app/src/server/telemetry/events.rs`.

```rust
TelemetryEvent::FileTreeReloadInvoked {
    duration_ms: u32,
}
```

A single field is sufficient for V1. If the reload fails, the duration is still recorded — the duration field plus the toast outcome (which the user sees) gives enough signal for usage analysis. A `success: bool` field can be added in V2 if failure rate proves interesting.

### 6. Iconography

**File:** `bundled/svg/refresh.svg` if it exists; otherwise add a small refresh-icon SVG to the bundle. If a refresh-style icon is already used by other reload actions in the palette (`Reload Settings`, etc.), use the same one for visual consistency.

## Testing and validation

Each invariant from `product.md` maps to a test at this layer:

| Invariant | Test layer | File |
|---|---|---|
| 1, 2 (palette presence/absence) | unit | new `app/src/search/command_palette/static_actions_tests.rs` extension — assert the action appears when `WORKSPACE_OPEN` is true and is absent otherwise. |
| 3 (re-scan picks up disk changes) | unit | `app/src/code/file_tree/reload_tests.rs` (new) — set up a temp project dir with one file, snapshot, add a second file on disk, call `reload_active_file_tree`, assert the snapshot now has both files. |
| 4 (works on non-Git directories) | unit | reload_tests — same as 3 but with a temp dir that has no `.git` subdir; assert the non-Git rebuild path is taken (mock or capture the call) and the snapshot updates. |
| 5 (success toast) | unit | dispatch_tests — invoke the action with a healthy workspace, assert `show_toast` is called with a message matching `^File tree reloaded \(\d+ ms\)\.$`. |
| 6 (failure toast + state preserved) | unit | dispatch_tests — invoke the action with a workspace whose root has been unmounted (mock the entry point to return Err); assert the toast surfaces the error and the in-memory snapshot is unchanged. |
| 7 (alias resolution) | unit | static_actions_tests — feed each of `refresh file tree`, `reload wildtree`, `refresh wildtree` to the palette search index, assert all three return the same action ID. |
| 8 (telemetry event) | unit | dispatch_tests — invoke the action, assert `FileTreeReloadInvoked` is emitted with a non-zero `duration_ms`. |

### Cross-platform constraints

- The reload path is platform-agnostic — both the Git-backed (`LocalRepoMetadataModel::rebuild_repository`) and the non-Git directory scan paths handle platform differences in their existing implementations.
- The toast/telemetry/palette infrastructure is shared across platforms.

## End-to-end flow

```
User opens Command Palette (Cmd+P / Ctrl+P)
  └─> palette renders results from registered sources           (existing)
        └─> static actions source includes RELOAD_FILE_TREE
              if Availability::WORKSPACE_OPEN holds

User types "reload file tree" / "refresh wildtree" / etc.
  └─> palette filters by alias-aware fuzzy match                (existing)

User selects the action
  └─> dispatch arm: PaletteActionId::ReloadFileTree             (new)
        ├─> started = Instant::now()
        ├─> reload_active_file_tree(ctx)                        (new)
        │     ├─> resolve active_workspace_root
        │     ├─> branch on is_git
        │     │     ├─> Git → LocalRepoMetadataModel.rebuild_repository
        │     │     └─> non-Git → SnapshotOwner.rebuild_from_disk
        │     └─> Ok(()) | Err(reason)
        ├─> elapsed = started.elapsed().as_millis()
        ├─> show_toast (success or failure variant)
        └─> send_telemetry FileTreeReloadInvoked { duration_ms: elapsed }
```

## Risks

- **Reload during active filesystem watcher event.** If the user invokes reload while the watcher is mid-update, two rebuild paths could race. **Mitigation:** the existing `LocalRepoMetadataModel` already serializes its rebuild work; explicit invocations queue behind in-flight ones rather than racing. The non-Git path needs the same guarantee — verify at implementation time, add a mutex if not already present.
- **Large project rebuild blocks the main thread.** `rebuild_from_disk` walks the project directory; for very large projects this could be slow. **Mitigation:** the existing automatic refresh already handles this via background tasks; explicit reload rides the same path. The toast shows the duration so users see what they got — if it's consistently > 1 second on real workspaces, the rebuild path itself is the optimization target, not this action.
- **Partial reload from a flaky disk.** If the project directory partially returns errors mid-walk (network mounts, permissions), the rebuild may produce a tree that's neither the old state nor the current disk state. **Mitigation:** rebuild is atomic at the model level — the new state replaces the old or the rebuild fails entirely. Surface the failure in the toast and preserve the prior state.
- **Telemetry on failed reloads is asymmetric with success.** Both currently emit `FileTreeReloadInvoked`; failure is signaled only by the toast. **Mitigation:** acceptable for V1; the visible toast is the primary user feedback. If failure rate becomes interesting, V2 adds a `success: bool` field.

## Follow-ups (out of this spec)

- A keyboard shortcut bound by default once usage data shows it's frequent.
- Per-folder reload action.
- Visible refresh button on the File Tree itself (explicitly rejected by the issue reporter; included here for completeness).
- Investigation of the underlying automatic-refresh failure that motivated this fallback.
- Reload-on-window-focus behavior change (would require careful UX validation).
