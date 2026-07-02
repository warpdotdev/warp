# PRODUCT.md - Auto-save for Code Editor

Issue: https://github.com/warpdotdev/warp/issues/10208

## Summary

Add VS Code-like auto-save behavior to Warp's built-in code editor, configurable
from Settings > Code. This spec focuses on auto-save; external file auto-reload
already exists and is not being redesigned here.

Figma: none provided.

## Goals / Non-goals

Goals:

- Add a user-visible auto-save setting with clear modes.
- Auto-save only when it is safe (no unresolved file-version conflict).
- Keep manual-save UX intact while reducing noisy save toasts for automatic saves.

Non-goals:

- Redesigning or replacing external file auto-reload behavior.
- Changing save semantics for notebooks or terminal input surfaces.
- Adding new background-save telemetry requirements in this spec.

## Behavior

1. Warp exposes a new Code setting named "Auto-save" with four user-facing
   options and stable persisted identifiers:
   - `Off` (persisted value: `off`)
   - `After Delay` (persisted value: `after_delay`)
   - `On Focus Change` (persisted value: `on_focus_change`)
   - `After Delay + On Focus Change` (persisted value:
     `after_delay_and_focus_change`)

2. The default auto-save value is `off`.

3. When `off` is selected, editor behavior remains manual-save only. Existing
   explicit save actions continue to work exactly as today.

4. When `after_delay` is selected, user edits in a code editor tab trigger an
   automatic save after a short debounce interval (approximately one second)
   once editing pauses.

5. When `on_focus_change` is selected, an automatic save is attempted when the
   editor loses focus (for example, the user clicks away to another pane/view).

6. When `after_delay_and_focus_change` is selected, both triggers are active:
   save-on-pause and save-on-editor-blur.

7. Auto-save only runs for editors backed by a real file. Unsaved/new buffers
   that require a save location do not silently create files.

8. Auto-save is skipped when there are no unsaved changes.

9. Auto-save is skipped when a version conflict is present (for example, file
   content changed on disk and the editor has diverged local edits). In this
   state, existing conflict resolution UX remains the source of truth.

10. Successful manual saves continue to show save success feedback exactly as
    today.

11. Successful auto-saves do not show the same manual "File saved" success toast,
    to avoid noisy repeated notifications while typing.

12. Auto-save does not implicitly run format-on-save/LSP formatting. Explicit
    manual save and format actions continue to control formatting behavior.

13. Failed auto-saves surface the same error behavior as failed manual saves.

14. Auto-save mode changes in Settings apply to newly opened and currently open
    code editors without requiring app restart. Delayed autosave callbacks read
    the current mode at fire time, and pending delayed autosaves are canceled/
    skipped when mode is changed to `off`.

15. The auto-save setting participates in normal settings persistence and sync
    semantics for Code settings.

16. Existing external file auto-reload behavior remains available and unchanged
    by this feature.
