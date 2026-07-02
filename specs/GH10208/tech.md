# TECH.md - Auto-save for Code Editor

Issue: https://github.com/warpdotdev/warp/issues/10208
Product spec: `specs/GH10208/product.md`

## Context

Current master has manual save behavior in the code editor and already performs
external file refresh when file-model updates arrive.

Relevant code paths:

- `app/src/settings/code.rs:5-63` defines `CodeSettings`; there is currently no
  auto-save mode setting.
- `app/src/settings_view/code_page.rs:286-311` builds the "Code Editor and
  Review" widget list; there is currently no auto-save control.
- `app/src/settings_view/code_page.rs:521-748` defines settings actions and
  handlers; no action currently writes an auto-save mode.
- `app/src/code/local_code_editor.rs:110-167` defines `LocalCodeEditorEvent`;
  `FileSaved` currently has no save-origin metadata.
- `app/src/code/local_code_editor.rs:1526-1528` emits `FileSaved` when the
  buffer model reports a save completion.
- `app/src/code/local_code_editor.rs:1549-1556` implements manual save entrypoint.
- `app/src/code/view.rs:520-524` always shows save success toast on `FileSaved`.
- `app/src/code/global_buffer_model.rs:390-446` already reacts to
  `FileModelEvent::FileUpdated` and refreshes buffers under safe conditions.

The issue thread explicitly confirms auto-reload already exists; this spec
targets auto-save behavior.

## Proposed changes

1. Add auto-save settings model in `app/src/settings/code.rs`.
   - Introduce `CodeAutoSaveMode` enum with values:
     `Off`, `AfterDelay`, `OnFocusChange`, `AfterDelayAndFocusChange`.
   - Add a new setting field under `CodeSettings` at TOML path
     `code.editor.auto_save.mode`.
   - Persist enum values as `off`, `after_delay`, `on_focus_change`,
     `after_delay_and_focus_change`.
   - Render user-facing dropdown labels as `Off`, `After Delay`,
     `On Focus Change`, `After Delay + On Focus Change` (labels are display
     strings; persisted values remain snake_case identifiers).
   - Set settings metadata explicitly:
     - `supported_platforms`: `SupportedPlatforms::ALL`.
     - `sync_to_cloud`: `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`.
     - `private`: false (non-secret preference).
   - Keep default at `Off`.

2. Add auto-save UI in `app/src/settings_view/code_page.rs`.
   - Add an "Auto-save" dropdown widget in the "Code Editor and Review" section.
   - Add a typed action that updates the new setting field.
   - Ensure dropdown reflects live setting updates and remains searchable.

3. Add autosave trigger execution in `app/src/code/local_code_editor.rs`.
   - Introduce a debounced autosave trigger channel driven from user-origin
     content changes.
   - Add a focus-change trigger on editor blur.
   - Gate execution by selected mode (`Off` / delay / blur / both).
   - Use a dedicated autosave path that skips format-on-save/LSP formatting and
     invokes a direct save operation.
   - Guard autosave with:
     - file-backed buffer present
     - unsaved changes present
     - no unresolved version conflict
   - Re-check conflict/version state immediately before dispatching each autosave
     (after debounce delay and after blur trigger).
   - Evaluate current auto-save mode at trigger fire time (not only at schedule
     time), and cancel/skip pending debounced autosaves when mode transitions to
     `Off`.

4. Enforce conflict safety at final write point in
   `app/src/code/global_buffer_model.rs`.
   - Extend save request plumbing to carry an expected file/buffer version from
     autosave trigger time.
   - Immediately before writing to disk, verify expected version still matches
     current model state; if mismatched, abort save as conflict/no-op.
   - On abort, preserve dirty/conflict state and emit an explicit save-skipped
     (conflict) outcome so existing conflict UX remains authoritative and no
     success path is observed.
   - Keep manual save behavior exactly unchanged: this expected-version final
     guard is autosave-only in this feature.

5. Differentiate manual vs automatic save origin.
   - Add save-origin metadata to local editor save completion events.
   - Add a per-save request correlation ID (`save_request_id`) generated at
     initiation and carried through the save pipeline and completion callback.
   - Match completions by `save_request_id` (not only file/version) so
     overlapping manual + autosave operations cannot cross-classify origin.
   - Track save origin through local save initiation and global buffer save
     completion callbacks.
   - Preserve existing behavior for manual saves.

6. Toast behavior update in `app/src/code/view.rs`.
   - Show save success toast only for manual saves.
   - Keep path/title synchronization and error handling unchanged for both
     manual and automatic saves.

7. Keep external auto-reload unchanged.
   - No behavioral changes to `GlobalBufferModel` file-update application logic
     are required by this spec.

## Testing and validation

Map to `product.md` behavior invariants:

- (1, 2, 14, 15) Settings model + UI:
  - Add/update settings-view tests asserting:
    - Auto-save dropdown renders in Code Editor and Review page.
    - Setting writes persist and UI reflects current selection.
    - Dropdown shows user-facing labels (not raw persisted snake_case values).
    - Defaults are `off`.

- (3) Off mode:
  - Unit test: user edit in `Off` mode does not trigger save call after debounce.
  - Unit test: switching mode to `Off` cancels/skips already-pending debounced
    autosave and delayed callback re-checks current mode before save dispatch.

- (4) After-delay mode:
  - Unit test in local editor model: user edit schedules one save after debounce.
  - Ensure multiple rapid edits coalesce into one save attempt.

- (5) Focus-change mode:
  - Unit test: blur triggers save when unsaved changes exist.
  - Unit test: blur does not trigger save when no unsaved changes.

- (6) Combined mode:
  - Unit test: both debounce and blur paths are enabled.

- (7, 8) Save preconditions:
  - Unit tests: autosave no-ops for non-file-backed/new buffers and no-unsaved
    state.

- (9) Conflict gate:
  - Unit test: autosave no-ops when version conflict exists at trigger time.
  - Unit test: autosave aborts when version changes between trigger and final
    write (debounce/async race window).
  - Unit test: aborted final-write guard preserves dirty/conflict state and
    returns save-skipped/conflict outcome (no success event).

- (5, 6, 10, 11) Save-origin correlation + toast behavior:
  - Unit test: overlapping manual and autosave requests on same file are
    correlated by `save_request_id` and produce correct origin classification.
  - Code view test(s): success toast suppression applies only to correlated
    autosave completions, while manual save completions still toast.

- (12, 13) Formatting + error behavior:
  - Unit/integration test: autosave path does not call format-and-save hooks.
  - Unit/integration test: autosave failure emits existing save failure event and
    surfaces existing error UI.

- (16) Auto-reload unchanged:
  - Regression check: existing tests around file update handling continue to pass.

Validation commands:

- `cargo fmt`
- `cargo check -p warp`
- Targeted tests for code settings page, local code editor autosave paths, and
  code view save-toast behavior.
- `./script/presubmit` before implementation PR (not required for spec-only PR).

## Risks and mitigations

- Risk: autosave introduces noisy UX via repeated toasts.
  - Mitigation: gate success toast to manual saves only.

- Risk: autosave attempts during conflict states may overwrite user intent.
  - Mitigation: skip autosave when version-conflict predicate is true.

- Risk: subtle behavior differences between save triggers.
  - Mitigation: centralize trigger gating and route both paths through one
    save entrypoint.

## Follow-ups

- If maintainers want explicit user control for existing auto-reload behavior,
  propose a separate issue/spec that builds on current `GlobalBufferModel`
  semantics without coupling to this auto-save feature.
