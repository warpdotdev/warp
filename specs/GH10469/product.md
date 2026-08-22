# Product Spec: Same-line layout for the Warp prompt

**Issue:** [warpdotdev/warp#10469](https://github.com/warpdotdev/warp/issues/10469)

## Summary

Restore an optional same-line layout for the Warp prompt so users can place the
Warp context prompt and terminal command input on one row. The option must work
with the current interactive context chips without changing the default prompt
layout or the behavior of shell (PS1) and agent inputs.

## Goals / Non-goals

Goals:

- Restore the compact prompt layout for both existing and newly configured Warp
  prompts.
- Preserve current chip content, ordering, interactions, and prompt settings.
- Keep command entry usable as chips and terminal width change.

Non-goals:

- Changing the shell prompt (PS1) layout or parsing shell prompt text.
- Moving the Agent Mode input footer or its toolbar chips onto the terminal
  command line.
- Redesigning, reordering, or changing the availability rules of context chips.
- Changing how command text itself wraps or how completed blocks are displayed.

## Figma

Figma: none provided.

## Behavior

1. The Warp prompt editor offers a setting named **Same line prompt** when the
   Warp prompt is selected. It is off for the default Warp prompt, so users who
   have never enabled it continue to see the prompt above the command input.

2. Enabling **Same line prompt** and saving places the visible Warp prompt
   immediately before the terminal command input. The prompt and the first line
   of command text share a baseline and read in left-to-right order as prompt,
   optional separator, then command text.

3. Previously saved Warp prompt configurations with same-line enabled begin
   using the restored layout without requiring the user to open or re-save the
   prompt editor. Previously saved configurations with it disabled, and the
   default Warp prompt, remain above the input. This compatibility applies to
   both the older terminal input presentation and the current Warp terminal
   input presentation.

4. Enabling or disabling the setting does not add, remove, reorder, or reset
   context chips. Every visible chip keeps its existing text, icon, color,
   tooltip, click target, menu behavior, and configured order.

5. When same-line is enabled, the prompt editor also offers the existing
   separator choices: no separator, `%`, `$`, and `>`. The selected separator
   appears once after the last visible prompt chip and before the command input.
   Choosing no separator leaves only the normal spacing between the prompt and
   input; it does not leave an empty glyph or duplicate gap.

6. The separator selection is editable only for a same-line Warp prompt. Its
   saved value is retained if same-line is temporarily disabled and is restored
   when same-line is enabled again. Resetting to the default Warp prompt resets
   same-line to off and the separator to none.

7. Disabling **Same line prompt** and saving returns the Warp prompt to its
   current above-input layout. The command being edited, selection, undo
   history, autosuggestion, and input focus are preserved while the layout
   updates.

8. Selecting the shell prompt (PS1) keeps PS1's existing behavior and does not
   expose an active Warp same-line control. Switching back to the Warp prompt
   restores the last saved Warp prompt layout; switching prompt types does not
   silently rewrite the saved same-line preference.

9. Same-line affects the active terminal command input only. Completed command
   blocks, block history, copied prompt text, shared block content, and prompt
   context-menu values continue to contain the same prompt information as the
   above-input layout.

10. Entering Agent Mode uses Agent Mode's existing input/footer layout even if
    same-line is enabled. Leaving Agent Mode restores the same-line terminal
    command prompt without changing the setting or the command text that was
    present before the transition.

11. When the pane is too narrow to show the configured prompt and a usable
    command input together, the prompt yields horizontal space to the editor:
    chip text may use its existing truncation treatment, but prompt content and
    command text never overlap or paint outside the input. The editor always
    retains enough visible space for the insertion cursor and adjacent command
    text. If that is not possible beside the prompt, the prompt temporarily
    uses the above-input layout. Widening the pane restores same-line
    automatically; it does not alter the saved setting.

12. Resizing a pane, opening or closing a side panel, and changing terminal font
    size reflows the prompt without flicker, stale blank rows, or cursor jumps.
    At every width, typed text and the insertion cursor remain visible.

13. Context changes update the same-line prompt in place. Chips that appear,
    disappear, or change value after a directory change, Git operation,
    environment activation, or asynchronous refresh do not clear the command,
    move focus, close an unrelated open input menu, or switch the prompt to the
    above-input layout unless the width rule in (11) requires it.

14. The setting applies consistently in local and SSH terminal sessions. An SSH
    session shows its current remote context chips on the same line, and moving
    between local and remote sessions does not reset the saved layout. A missing,
    delayed, or failed chip value is simply omitted under its existing rules.

15. During shell bootstrap or prompt refresh, any existing loading message uses
    the same placement decision as the prompt it replaces. Replacing that
    message with live chips does not introduce an extra blank line or steal
    input focus.

16. The **Same line prompt** control is operable with mouse and keyboard. It has
    a visible focus treatment; `Tab` and `Shift-Tab` reach and leave it in modal
    order; `Space` toggles it; and clicking its visible label also toggles it.
    `Escape` still cancels the modal and saving still requires the existing Save
    action.

17. Assistive technology identifies the control as a checkbox, announces its
    **Same line prompt** label and checked state, and announces the new state
    after a toggle. The separator control keeps its existing label and disabled
    state semantics. Changing either control does not move focus unexpectedly.

18. Cancelling the prompt editor discards same-line and separator edits. Saving
    applies them to all open terminal panes that use the Warp prompt and to new
    panes, and the existing prompt-settings synchronization behavior is
    preserved across app restarts and signed-in devices.
