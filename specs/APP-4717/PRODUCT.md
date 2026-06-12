# Enter on empty input sends the top queued prompt

Linear: [APP-4717](https://linear.app/warpdotdev/issue/APP-4717/change-it-so-hitting-enter-w-an-empty-buffer-and-queued-prompts-auto)

## Summary

When the queued prompts panel is showing and the terminal input is empty, pressing Enter sends the top queued row immediately — exactly as if the user clicked that row's Send-now button. The panel header advertises this with a "⏎ to send" hint that appears only when Enter would actually send.

Figma: none provided. Reference is Cursor's queue UI, which shows an Enter-to-send hint next to its "N Queued" header label; we copy that placement in our panel header.

## Behavior

1. With the queued prompts panel visible, the input buffer completely empty, and the top queued row sendable (see 5), pressing Enter in the terminal input sends the top row immediately. This is identical to clicking that row's Send-now button:
   - An agent prompt row is submitted to the same target Send-now would use (running conversation follow-up, cloud follow-up, shared-session viewer send, or full-terminal-use agent when one is in control), carrying the row's own queued attachments.
   - A command row (`!` prefix) executes as a shell command.
   - The row is removed from the queue after dispatch; remaining rows shift up.
2. This applies regardless of the input's mode. In shell mode, an empty-buffer Enter that previously produced a fresh prompt line instead sends the top queued row while the panel is showing a sendable top row.
3. Enter sends exactly one row per press. Pressing Enter again re-evaluates: if the new top row is sendable and the buffer is still empty, it sends that row next.
4. The behavior works the same whether the panel body is expanded or collapsed (the header is visible either way).
5. Enter mirrors Send-now availability for the top row. While the top row's Send-now is disabled — the locked initial cloud-mode prompt while cloud environment setup is in progress — Enter does nothing and the hint is hidden.
   - The same applies whenever prompt sending is unavailable for the pane as a whole, e.g. the user is a read-only (non-executor) viewer in a shared session. In that state the rows' Send-now buttons are also disabled (with a tooltip explaining why), Enter does nothing, and the hint is hidden — button and Enter availability are bundled and may not disagree.
   - Pane-level unavailability only affects sending: it does not disable a row's edit/delete buttons, and it does not stop new prompts from being queued.
   - Enter-only conditions (non-empty buffer, CLI-agent rich input open) hide the hint and suppress Enter but do not disable the Send-now buttons.
6. If the buffer contains any content (including whitespace-only content), Enter behaves exactly as it does today and the hint is hidden.
7. Header hint:
   - The panel header shows an ⏎ keycap followed by "to send" next to the "N queued" label, matching the look and spacing of Warp's existing keystroke hints (e.g. "? for help"). The "to send" text uses the same color as the "N queued" label; the keycap glyph is dimmer (disabled-text styling) so it reads as a secondary affordance.
   - The hint is visible exactly when an empty-buffer Enter would send the top row, and hidden otherwise (non-empty buffer, no sendable top row, sending unavailable per 5, panel hidden, or any case in 8–10). The hint and the Enter behavior must never disagree.
8. Whenever the panel is not rendered (no queue, inline menu like slash commands or the model selector is open, feature flag off), Enter keeps its existing behavior and no hint is shown.
9. While a queued row is in inline edit mode, Enter commits that edit as today (focus is in the row's editor, not the input). The header hint is hidden during an inline edit.
10. When the CLI-agent rich input is open, Enter keeps its existing submit-to-CLI-agent behavior and the hint is hidden. (The `submit_on_ctrl_enter` setting only affects the CLI-agent rich input, so it never changes which key sends a queued row.)
11. Sending via Enter does not touch the input buffer, its pending attachments, or focus: the buffer stays empty, attachments staged in the input are not consumed (the queued row carries its own), and focus remains in the input afterward. One pre-existing exception, shared with the Send-now button: on the shared-session viewer path an empty input temporarily shows the standard "<prompt> ◌" loading affordance until the sharer acknowledges the send.
12. When the last queued row is sent, the panel disappears (existing behavior); a subsequent Enter behaves as it did before this feature.
13. Sending a queued row records telemetry distinguishing the trigger: Send-now button click vs. Enter on empty input.
