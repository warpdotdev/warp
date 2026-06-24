# Add Enter-to-send-now for queued prompts

Linear: [QUALITY-885](https://linear.app/warpdotdev/issue/QUALITY-885/add-enter-to-send-now-action-for-queued-prompts)

Related: `specs/APP-4717/` describes the same interaction under an earlier app ticket; this spec is the QUALITY-ticket source of truth.

## Summary
When the queued prompts panel is visible and the terminal input is empty, pressing Enter sends the top queued row immediately, matching the row's Send-now button. The panel header shows an `⏎ to send` hint only when pressing Enter would actually send a queued row.

Figma: none provided. The interaction follows Cursor's queue UI pattern: an Enter-to-send hint next to the `N queued` header label.

## Problem
Users can already queue prompts while an agent is busy, but sending the next queued prompt early requires discovering and clicking a hover-only Send-now button. An empty input is otherwise idle in this state, so Enter should be a keyboard equivalent for sending the next queued row.

## Behavior
1. When the queued prompts panel is visible, the terminal input buffer is completely empty, prompt sending is available for the pane, the CLI-agent rich input is closed, and the top queued row is sendable, pressing Enter in the terminal input sends the top queued row immediately.
2. Sending via Enter is equivalent to clicking the top row's Send-now button:
   - Agent prompt rows submit to the same target Send-now would use, including the active local conversation, a cloud follow-up, a shared-session executor, or the full-terminal-use agent when that agent is in control.
   - Prompt rows carry the attachments captured on that queued row, not any currently staged attachments in the input.
   - Command rows execute as shell commands.
   - The fired row is removed from the queue after dispatch, and remaining rows shift up.
3. Enter sends exactly one queued row per keypress. A second Enter re-evaluates the new state and sends the new top row only if all send conditions still hold.
4. The behavior is independent of the panel body state: a collapsed panel still supports Enter-to-send because the header remains visible and still advertises the shortcut when available.
5. The behavior is independent of the input's current shell/agent mode. In shell mode, an empty-buffer Enter sends a queued command or prompt instead of submitting an empty shell command when the send conditions hold.
6. Enter mirrors Send-now availability for the top row. If the top row's Send-now button is disabled, Enter does not send it and the header hint is hidden.
7. The locked initial cloud-mode row is not sendable. While it is at the head of the queue, Enter does nothing even if sendable rows exist behind it, and the hint is hidden. Once lifecycle events remove the locked row, Enter can send the next row if it is otherwise sendable.
8. Read-only shared-session viewers and any other pane state where prompt sending is unavailable cannot send queued rows via Enter. In those states, row Send-now buttons are disabled with explanatory tooltips, Enter does nothing, and the hint is hidden. Edit, delete, and reorder behavior remain governed by their own existing availability rules.
9. If the input buffer contains any content, including whitespace-only content, Enter keeps its existing behavior and does not send a queued row. The queued row stays in place and the header hint is hidden.
10. If the queued prompts panel is not rendered, Enter keeps its existing behavior and no hint is shown. This includes no queue, feature flag off, and inline menus such as slash commands, the model selector, prompts, skills, conversations, history, repos, plans, and context menus.
11. If a queued row is in inline edit mode, Enter commits that edit through the row editor's existing behavior. The header hint is hidden while any row is being edited.
12. If the CLI-agent rich input is open, Enter keeps its existing CLI-agent input behavior and does not send a queued panel row. The header hint is hidden in this state.
13. The header hint appears next to the `N queued` label as an Enter keycap followed by `to send`. It uses Warp's existing keycap rendering, spacing, and theme-derived colors: the text matches the queued label color, and the keycap glyph is visually dimmer so the shortcut reads as secondary guidance.
14. The hint is visible exactly when pressing Enter would send the top queued row. The hint and the actual Enter behavior must never disagree.
15. Sending via Enter does not consume or alter the input buffer, pending input attachments, or input focus. The buffer remains empty and focus remains in the input after dispatch. The shared-session viewer send path may temporarily show the existing `<prompt> ◌` loading affordance when the input is empty, matching Send-now behavior.
16. When Enter sends the last queued row, the panel disappears through existing empty-queue behavior. A subsequent Enter behaves as it did before any queued rows were present.
17. Telemetry records immediate queued-row sends with enough detail to distinguish Send-now button clicks from Enter-on-empty-input sends.
