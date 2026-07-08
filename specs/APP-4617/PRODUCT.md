# Product Spec: Attachments on Queued Prompts

Linear: [APP-4617](https://linear.app/warpdotdev/issue/APP-4617)
Figma: none provided

## Summary
Queued agent prompts retain the image and file attachments staged in the input at the time they were queued. The attachments move off the input onto the queued row, are sent with that prompt when it fires, and are restored to the input if the row is brought back for editing.

## Problem
With queued prompts, staged attachments lived only in the live input. Queuing a prompt either left them in the input (where they would be re-sent with the user's next prompt) or cleared them (losing them entirely). A queued prompt could never carry its own attachments, and a fired queued prompt could steal attachments meant for the next message.

## Behavior
1. When a prompt is queued while one or more attachments (images, files) are staged in the input, those attachments are captured onto the queued row and removed from the live input. The input returns to empty — no text and no attachment chips — ready for the next prompt.
2. Each queued row owns its own attachment set. Queuing prompt A with attachment X and then prompt B with attachment Y produces two rows that each carry only their own attachments; firing or removing one row never alters the other's attachments.
3. When a queued row fires (the conversation it was queued on finishes), the prompt is sent with that row's stored attachments: images are sent inline as image context, and files are sent as file references. Files that share a basename are disambiguated with `(1)`, `(2)`, … suffixes.
4. A fired queued row is always submitted into the conversation it was queued on, even if the user has since navigated to a different conversation. Its attachments resolve from the row, never from whatever is currently staged in the input.
5. Attachments staged for the user's next prompt are never consumed by a firing queued row. If the user stages new attachments after queuing, those stay in the input and are sent with the user's next prompt, independent of any row that fires in the meantime.
6. Restoring a queued row to the input re-stages its attachments (the chips reappear):
   - If the head row was in edit mode when auto-fire reached it and the input is empty, the row's last-committed text and its attachments are placed back into the input. Uncommitted live-editor text is not used.
   - When the user manually restores a row to the input for editing, its attachments are re-staged so a manual re-submit keeps them.
7. Removing a queued row — whether deleted by the user or removed after it fires — drops its attachments. They are not left behind in the input or any other row.
8. Queued slash commands and queued skill invocations carry attachments the same way: a queued skill invocation with staged images/files sends them with the skill when it fires. A direct (non-queued) skill invocation continues to consume the live input staging as before.
9. The `/queue` command, when the agent is not currently in progress, submits immediately as a regular prompt — consuming and clearing the live staging — rather than being treated as a queued-row fire.
10. **Known limitation — cloud follow-up:** when a queued row fires into a cloud follow-up prompt, which does not support attachments, the prompt text is still sent but the row's attachments are dropped (a warning is logged). No attachment chips or files reach the cloud run.
11. **Shared-session viewer:** when a queued row fires in a shared-session viewer, its stored images/files are uploaded and sent with the prompt (when the cloud pane supports image context), via the same upload-then-send path as an immediate viewer submission.
12. Empty-queue and locked-row behavior is unchanged: draining an empty queue does nothing, and the locked initial Cloud Mode row never auto-fires. Each conversation keeps an independent queue, so attachments on one conversation's rows never appear on another's.
