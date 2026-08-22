# Block navigation in agent mode

Tracking: [#9959](https://github.com/warpdotdev/warp/issues/9959)

## Summary

Bring Warp's terminal-mode block navigation (CMD-UP / CMD-DOWN to select the most recent block, arrow keys to move between blocks, SHIFT-CMD-UP/DOWN to scroll to the edges of the selected block) to agent mode. In agent mode, navigable units are agent responses, agent-executed commands (with their output), and user prompts — and which of those count as "blocks" is configurable in Settings.

## Problem

In long agent conversations — multiple back-and-forths, several agent-run commands with lengthy output, large markdown responses — the only way to revisit an earlier exchange is manual scrolling. Terminal mode does not have this problem because every block is keyboard-navigable. The keyboard model that makes terminal mode pleasant simply doesn't exist on the agent side, so power users lose the same workflow exactly when conversations get long enough to need it most.

## Goals / Non-goals

**Goals**
- Same muscle memory as terminal mode: CMD-UP / arrow keys / SHIFT-CMD-UP-DOWN behave the same way wherever you are in Warp.
- A single configurable surface for which agent-mode element types participate in block navigation.
- Selection in agent mode is at parity with terminal mode for copy, plus useful agent-specific affordances (attach as context, rerun command, retry response, fork from prompt).

**Non-goals**
- Redesigning terminal-mode navigation, keybindings, or selection styling.
- Navigating *within* a single agent response (heading-by-heading, code-block-by-code-block). The smallest navigable unit is the whole response.
- Rebinding CMD-UP / CMD-DOWN / arrow keys in agent input itself when the input field has focus and a typical text-editing operation is expected.

## Figma

Figma: links to be added by the spec author before the spec is approved. Until then, agent-mode block selection MUST visually match terminal-mode block selection (same border treatment, same theme tokens, same multi-select rendering).

## Behavior

### Block model

1. In an agent-mode conversation, the navigable items in left-to-right reading order are the visible *agent blocks* in the conversation:
   - **User prompts** — each message the user sent to the agent.
   - **Agent responses** — each top-level response the agent produced.
   - **Agent-executed commands** — each command the agent ran, *together with its output as a single block* (the command and its output are not separately navigable).
2. When agent mode is rendered inline inside a terminal block list (agent view is embedded rather than full-screen), all blocks in that block list — both terminal blocks and agent blocks — participate in the same navigation order. The user perceives one continuous block list, not two.
3. When agent mode is rendered full-screen, only agent blocks are present and only they participate in navigation.
4. A block that is still streaming (response being generated, or command still producing output) IS navigable and selectable. Its visible bounds may grow while selected; the selection remains attached to the same logical block as it grows.
5. A tool-call block that has not yet produced any visible content (zero-output command, in-flight tool call with nothing rendered yet) IS still navigable — empty does not mean absent.
6. Hidden / collapsed blocks (folded responses, collapsed command output) are skipped during arrow navigation. Expanding a previously hidden block makes it navigable again. Selection state is preserved across collapse/expand of the *selected* block.

### Configurable block-type filter

7. A setting under **Settings → AI → Block navigation** controls which agent-block types participate in CMD-UP / arrow navigation. The setting is a multi-select with three independent toggles:
   - Agent responses (default: on)
   - Agent-executed commands (default: on)
   - User prompts (default: on)
8. The default ships with all three on. With all three on, every agent block is navigable.
9. Disabling a type via the setting causes navigation to **skip** blocks of that type, exactly as if they were not in the list. Disabled-type blocks remain visible in the conversation; only navigation skips them.
10. Disabling all three types disables agent-mode block navigation entirely. CMD-UP and arrow keys then fall through to whatever lower-priority binding would otherwise handle them in agent mode (typically the agent input field for arrow keys; nothing for CMD-UP).
11. Changing the setting mid-conversation takes effect on the next keypress without requiring a refresh. If the currently selected block becomes a non-navigable type because of a setting change, selection is cleared.
12. The setting is per-user (synced via the same mechanism as other Settings → AI preferences), not per-conversation.

### Selecting a block — initial selection

13. With no agent block currently selected, pressing **CMD-UP** while the agent view has focus selects the most recent navigable agent block (the bottom-most block in the conversation, after applying the block-type filter from §7). The selected block is scrolled into view if not already visible.
14. With no agent block currently selected, pressing **CMD-DOWN** while the agent view has focus is a no-op — there is nothing below the input.
15. CMD-UP / CMD-DOWN never move focus into an editable input; they always operate on the block list. (Contrast with arrow keys, which only operate on blocks when a block is already selected — see §17.)

### Moving between blocks

16. With a block selected, **UP** / **DOWN** arrow keys move the selection to the previous / next navigable agent block (per the §7 filter), scrolling the new selection into view.
17. With **no** block selected, UP / DOWN arrow keys retain their default behavior in whatever input or surface currently has focus. Arrow keys do not "wake up" block selection — only CMD-UP does.
18. Pressing UP at the top-most navigable block leaves selection on that block (no wrap-around).
19. Pressing DOWN at the bottom-most navigable block clears selection and returns focus to the agent input field, mirroring the equivalent terminal-mode behavior.
20. **CMD-UP / CMD-DOWN** with a block already selected jumps to the top-most / bottom-most navigable block in the conversation (jump to extremes), not just one step.
21. **SHIFT-UP / SHIFT-DOWN** extends the selection by one navigable block in that direction, producing a multi-block selection. **SHIFT-CMD-UP** with no selection selects from the bottom of the conversation up to the top-most navigable block.

### SHIFT-CMD scroll

22. **SHIFT-CMD-UP** scrolls the viewport so the top edge of the selected block (or the top-most block in a multi-selection) is visible. **SHIFT-CMD-DOWN** scrolls so the bottom edge of the selected block (or bottom-most block in a multi-selection) is visible. Selection itself is not changed. This matches terminal-mode `ScrollToTopOfSelectedBlocks` / `ScrollToBottomOfSelectedBlocks` semantics.
23. With nothing selected, SHIFT-CMD-UP / SHIFT-CMD-DOWN are no-ops in agent mode.

### What selection does

24. A selected block renders with the same selection border treatment as a selected terminal block — same border width, same theme tokens, same multi-block range rendering — so users perceive one consistent "selected block" UI across modes.
25. **Copy (CMD-C)** with one or more agent blocks selected copies their content to the system clipboard:
    - User prompt → the prompt's plain text.
    - Agent response → the response's markdown source (not the rendered HTML/styled form).
    - Agent-executed command → the command line followed by its output (same shape as copying a terminal block).
    Multi-block copy concatenates blocks in conversation order with a single blank line between them.
26. **Attach to next prompt as context** is exposed as an explicit action — keystroke `CMD-SHIFT-K` and a button on the selected block's hover affordance — *not* automatically on selection. Triggering it adds the selected block(s) as a context chip on the agent input. The chip is removable like any other context chip, and the user can keep navigating / selecting more blocks and attach them too.
27. **Per-block actions** are exposed via the existing block hover/keyboard affordance (the same "block actions" surface terminal blocks already use). Available actions depend on the selected block's type:
    - User prompt → **Fork conversation from here** (creates a new conversation seeded with history up through but not including this prompt; the original conversation is unchanged).
    - Agent response → **Retry from here** (re-runs the agent with the same prompt that produced this response, replacing this response and everything after it with the new one).
    - Agent-executed command → **Rerun command** (re-executes the command in the same shell context the agent used; output is appended to the conversation as a new agent-command block).
28. Per-block actions in §27 are only available when exactly one block is selected. With a multi-block selection, only Copy and Attach-as-context are available.
29. Selection has no side effect beyond §24–§28. It does not edit conversation state, does not pause streaming, does not consume tool-call results, does not change agent focus.

### Focus and input interactions

30. When the agent input field has focus and a block is selected, typing any printable character (or any text-editing key like Backspace) clears the block selection and the keystroke goes to the input field. Modifier-key combinations and arrow keys are not "printable" for the purpose of this rule and do not clear selection.
31. Pressing **Escape** with a block selected clears the selection and returns focus to the agent input. Pressing Escape with no selection retains its existing agent-mode behavior.
32. Block selection is cleared whenever the user sends a new prompt to the agent. (The new exchange becomes the bottom of the list and any prior selection would be visually disorienting.)
33. Switching between agent mode and terminal mode does not preserve agent-mode selection — selection is per-mode and resets when the surface changes.
34. Block selection is not persisted across app restarts or conversation reloads.

### Edge cases

35. **Empty conversation** — no agent exchanges yet: CMD-UP and arrow keys are no-ops in agent mode. Nothing to select.
36. **Single-block conversation** — exactly one navigable block: CMD-UP selects it; UP/DOWN are no-ops; CMD-DOWN clears selection per §19.
37. **Streaming response selected** — if the user selects an agent response that is still streaming, navigation behavior is unchanged. Copy (§25) copies whatever is rendered at the moment of copy (no waiting for stream completion). Retry-from-here (§27) cancels the in-flight stream and starts a new generation.
38. **Long-running command selected** — equivalent to §37: copy reads currently-rendered output; rerun starts a new execution and does not interfere with the original.
39. **Block deleted while selected** (e.g. agent retracts a tool call, conversation is edited): selection moves to the next-newer navigable block. If no newer block exists, selection moves to the next-older navigable block. If no navigable block remains, selection is cleared.
40. **Filter change clears non-matching selection** — see §11.
41. **Input mode quirks** — terminal mode has an `InputMode::PinnedToTop` that inverts UP/DOWN semantics. Agent mode does NOT honor that mode; CMD-UP always means "select the most recent block" (i.e. the bottom-most), independent of any terminal-side pinned-to-top setting.
42. **AltScreen** — terminal-mode navigation is disabled when AltScreen is active. Agent mode is not affected by AltScreen state — the agent view is its own surface and its block navigation works regardless of any concurrent terminal AltScreen.

### Accessibility

43. Selecting a block via keyboard moves the assistive-tech focus to that block. The block's accessible name announces its type ("user prompt", "agent response", "agent command") and a short content preview (first ~80 characters of the block's text).
44. The selection border is rendered using existing theme tokens that satisfy the contrast requirements already met by terminal-mode block selection — no new tokens are introduced for agent mode.

### Cross-surface consistency

45. The keystroke chord, the selection visual, the multi-select extension behavior, and the "scroll to edge of selected block" behavior MUST be identical in semantics between terminal mode and agent mode. A user who learned the gesture in one mode applies it unchanged in the other. The only intentional differences between the two modes are the configurable block-type filter (§7–§12), the per-block-type action set (§27), and the absence of `PinnedToTop` inversion in agent mode (§41).
