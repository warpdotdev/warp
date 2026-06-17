# APP-3781: Move Plugin Installation Instructions into a Split Pane

Linear: [APP-3781](https://linear.app/warpdotdev/issue/APP-3781/move-plugin-installation-settings-into-a-dedicated-pane)

## Summary

Replace the blocking modal that shows CLI agent plugin install/update instructions with a split terminal pane containing a specialized zero state block. The pane is a real terminal session, so the user can read the instructions and run the commands without switching context.

## Problem

The current plugin install/update instructions are shown in a modal overlay that blocks the entire screen. The user must:
1. Read a step in the modal
2. Copy the command
3. Close the modal
4. Paste the command into the terminal
5. Re-open the modal to see the next step

This is particularly frustrating because the instructions involve multiple sequential commands that the user needs to run in their terminal.

## Goals

- Show plugin install/update instructions in a side-by-side terminal pane instead of a blocking modal.
- Let the user run the instruction commands directly in the new pane without context switching.
- Preserve the existing "copy" affordance for each command step.
- Apply to both install and update instruction flows.

## Non-goals

- Changing the auto-install/auto-update behavior (the one-click install/update chip continues to work as before).
- Modifying the instruction content itself (titles, subtitles, step descriptions, commands remain unchanged).
- Adding "run in terminal" buttons that auto-execute commands — the user pastes/types them manually.

## User Experience

### Entry point

The entry points remain the same buttons in the CLI agent toolbar:
- The install chip (when in the instructions state) → opens install instructions pane.
- The update chip (when in the instructions state) → opens update instructions pane.

### What happens on click

1. A new terminal pane opens as a split in the current tab (using smart split direction, like env var panes do). This is always a terminal pane, not an agent pane, even if the user's default mode for new sessions is Agent Mode.
2. A specialized zero state block is rendered at the top of the new terminal's block list, displaying the plugin instructions.
3. The terminal session in the new pane is fully functional — the user can type and run commands.

### Instructions block content

The instructions block renders the same information as the current modal:
- **Title** (e.g., "Install Warp Plugin for Claude Code")
- **Subtitle** (e.g., "Ensure that jq is installed on your machine. Then, run these commands inside your Claude Code session.")
- **Numbered steps**, each with:
  - A step number in a circular badge
  - A text description of the step
  - A code block with the command and a copy-to-clipboard button

The visual style should match the existing terminal zero state block pattern: bordered container with terminal-consistent styling. Step rendering reuses the same code block rendering pattern from the modal (the `render_code_block_plain` helper).

### Dismissing the instructions block

The instructions block has a close button (X) in the top-right corner. Clicking it hides the block. The instructions block persists across commands — it does not auto-dismiss when the user runs a command, since the user may be following the multi-step instructions in that pane.

### Closing the pane

The user can close the instructions pane like any other split pane (via the pane close button, keyboard shortcut, etc.). No special cleanup is needed.

### Modal removal

The modal (`PluginInstallModal`) is removed entirely. All references to `is_plugin_install_modal_open` in workspace state are cleaned up.

## Edge Cases

1. **Single-pane tab**: If the tab has only one pane, the split creates a second pane. The instructions block appears in the new (right/bottom) pane.
2. **Already-split tab**: The new instructions pane is added as a sibling of the focused pane in the smart split direction, consistent with how env var panes split.
3. **Multiple instruction requests**: Clicking the instructions button again always opens a new split pane (no deduplication).
4. **Pane closed, re-requested**: If the user closes the instructions pane and clicks the button again, a new instructions pane is created from scratch.

## Success Criteria

1. Clicking the info (ⓘ) button next to the install/update chip opens a split terminal pane, not a modal overlay.
2. The new pane shows a zero state block with the full plugin instructions (title, subtitle, numbered steps with copy-able commands).
3. The user can type and run commands in the new pane while the instructions block is visible.
4. The instructions block persists until the user clicks the close (X) button on it.
5. The copy button on each step copies the command to the clipboard and shows a "Copied to clipboard" toast.
6. The modal overlay (`PluginInstallModal`) is fully removed from the codebase.
7. Both install and update instruction flows use the new split pane behavior.

## Validation

- **Manual test**: Click the install info button → verify a split pane appears with instructions. Copy a command → verify clipboard. Run a command in the pane → verify the instructions block remains visible. Click the close (X) button on the block → verify it disappears. Close the pane → verify clean closure.
- **Both flows**: Verify both install and update info buttons open the pane with the correct instructions.
- **Compile check**: Verify no remaining references to the removed modal types.

## Open Questions

(None outstanding.)
