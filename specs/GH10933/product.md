# Edit sent agent messages and regenerate
## Summary
Users can edit a prompt they already sent to a Warp agent and regenerate the agent response from the edited prompt. The edited prompt replaces the original prompt in the conversation timeline, and Warp removes and regenerates the agent output and any later conversation turns that depended on the original prompt.
## Problem
Today, a small typo, missing detail, or unclear sentence in a sent agent prompt requires a corrective follow-up or retyping the entire prompt. That workaround adds friction, creates noisy conversation history, and can make the agent reason from both the mistaken prompt and the correction instead of from the intended prompt alone.
## Goals
1. Let users correct a sent user prompt without manually copying and resubmitting it.
2. Make the conversation state after editing match the state the user would have gotten if they had sent the edited prompt originally.
3. Reuse familiar prompt editing patterns from Warp: explicit menu actions, keyboard accessibility, save/cancel affordances, and clear destructive-regeneration feedback.
4. Preserve existing safety expectations around cancelling active agent work, reverting generated file edits, and not mutating read-only transcripts.
## Non-goals
1. Editing agent responses, tool-call output, terminal command output, or child-agent messages authored by someone else.
2. Preserving the existing agent response after a prompt edit. The primary behavior is regenerate-from-edit; a future enhancement may offer “edit without regenerate.”
3. Editing attached files, images, or attached block selections as part of the initial feature. Existing attachments remain associated with the edited prompt unless a later design explicitly adds attachment editing.
4. Full visible version history for edited prompts. A recovery affordance can rely on existing fork/rewind backup mechanics, but the main conversation should show the edited prompt as the current prompt.
## Figma
Figma: https://www.figma.com/design/3XTgwiTkNQI2byxsBY00cG/Edit-messages-sent-to-the-agent?node-id=47-2791&t=rwW7jRBJHfGwyXql-1
Loom: https://www.loom.com/share/0725659433294c45aff89f3670189a49
## Behavior
1. Every editable sent user prompt in an agent conversation exposes an “Edit message” action from the prompt block’s existing overflow/context menu.
2. A prompt is editable only when all of these are true:
   - The prompt was authored by the current user in a mutable local Warp agent conversation.
   - The block is not a read-only transcript, imported debug transcript, shared-session viewer transcript, or restored surface that cannot safely mutate local conversation state.
   - The prompt belongs to a user query that can be regenerated as an active agent request.
3. A prompt is not editable when it represents agent output, tool results, passive suggestion output, read-only conversation search output, or a child/teammate message that the current user did not author.
4. Selecting “Edit message” changes the selected prompt block into an inline editing state in place. The editor is prefilled with the sent prompt text exactly as the user sees it, including slash-command prefixes such as `/plan` when those prefixes were part of the user-facing prompt.
5. While editing, the block shows clear actions to save/regenerate or cancel. The primary action is labeled to communicate that submitting the edit will regenerate the response, not only update text.
6. The inline editor supports multiline prompt text, soft wrapping, text selection, standard clipboard operations, and keyboard submission/cancellation.
7. Pressing Escape or clicking Cancel exits editing state without changing the prompt, response, downstream conversation turns, attached context, usage, or files.
8. Submitting an empty or whitespace-only edit is blocked. Warp keeps the editor open and shows a lightweight validation state instead of deleting the message.
9. Submitting unchanged text exits editing state without regenerating and without changing conversation history.
10. Submitting changed text replaces the original prompt with the edited prompt in the conversation timeline.
11. After a changed edit is submitted, Warp removes the original agent response for that prompt and removes all later user prompts, agent responses, actions, suggestions, and generated artifacts in that same conversation branch that depended on the original prompt.
12. Warp then sends the edited prompt as the next active agent request from the conversation state immediately before the edited prompt.
13. The regenerated response streams in the same place the old response occupied after the downstream blocks have been removed. From the user’s perspective, the conversation now looks as if the edited prompt had been sent originally.
14. If the edited prompt was the first prompt in the conversation, the conversation’s visible title/initial query reflects the edited prompt after regeneration begins.
15. If the edited prompt was a middle prompt, all conversation context before that prompt is preserved, and all context from that prompt onward is replaced by the regenerated path.
16. If the original prompt had attached files, images, selected text, or attached blocks, those attachments remain attached to the edited prompt for this initial version. The user can change the text around those references, but the edit UI does not provide add/remove attachment controls.
17. If the original prompt used a mode such as plan, orchestrate, create environment, invoke skill, or another prompt-like agent input, editing preserves that mode unless the user edits the visible slash-command prefix in a way that normal prompt submission would interpret differently.
18. If an agent response is currently streaming for the edited prompt or for a later prompt in the same conversation, submitting the edit stops the in-flight response before removing and regenerating downstream content.
19. If generated file edits or other reversible side effects exist in the removed portion of the conversation, Warp restores the workspace to the pre-edit state using the same user-facing safety model as rewinding before regenerating.
20. If side effects cannot be fully reverted automatically, Warp blocks regeneration or shows a confirmation/error state rather than silently leaving the workspace inconsistent with the regenerated conversation.
21. If regeneration fails, the edited prompt remains visible and the new response block shows the normal failed-response UI. The old prompt and old response are not silently restored unless the user uses an explicit recovery action.
22. If the user goes offline or the request cannot be sent after submitting the edit, Warp keeps the edited prompt and shows the normal request failure/offline state for the regenerated response.
23. Editing a prompt must not leak hidden or redacted secret content. Redaction, link detection, find-in-block, copy, and selection behavior for the edited prompt match normal sent prompt rendering after the edit is saved.
24. Copying the prompt after regeneration copies the edited prompt. Copying the full conversation copies the edited timeline, not the discarded pre-edit text.
25. The existing “Fork” and “Rewind” behaviors remain available and distinct from editing. Editing is for replacing a sent prompt and regenerating; Fork is for branching; Rewind is for removing conversation state without immediately resubmitting an edited prompt.
26. Keyboard access should exist for the edit action. The exact default shortcut is a design decision, but the shortcut should be discoverable in the same place as adjacent prompt/block actions and should not conflict with text-editing shortcuts.
27. The edit UI works in both classic blocklist agent conversations and Agent View wherever the same sent prompt block is rendered and mutable.
28. The feature should be safe to roll out behind a feature flag so dogfood users can validate the destructive-regeneration UX before broad release.
## Success criteria
1. A user can send a prompt, edit it from the prompt UI, submit the edit, and see a regenerated response based on the edited prompt without manually copying or retyping the original prompt.
2. Editing a middle prompt removes downstream conversation turns and regenerates from the pre-edit context without leaving stale blocks, stale pending actions, or stale generated-file state.
3. Cancel, unchanged submit, empty submit, request failure, active streaming cancellation, and read-only transcript states behave predictably and do not corrupt conversation history.
4. Users can discover and operate the edit flow with mouse and keyboard.
5. Existing copy, link detection, secret redaction, sharing/debugging IDs, fork, rewind, and response rating behavior continue to work for regenerated conversations.
## Validation
1. Manually verify happy-path editing of the most recent prompt in a new agent conversation.
2. Manually verify editing an earlier prompt after at least one follow-up removes downstream turns and regenerates from the edited prompt.
3. Manually verify editing while the response is streaming cancels the old stream and starts a new one.
4. Manually verify Cancel, unchanged submit, and empty submit do not mutate history.
5. Verify read-only/shared transcript surfaces do not expose an enabled edit action.
6. Verify any automated integration coverage checks the user-visible flow: send prompt, open edit, submit changed prompt, old response removed, new response appears.
## Open questions
1. What exact keyboard shortcut should trigger editing the most recent editable sent agent message when the input buffer is empty?
2. Should the first release include a visible “restore pre-edit version” affordance, or is the existing fork/rewind backup mechanism sufficient for dogfood?
3. Should attachment editing be prioritized immediately after text editing, or remain a separate follow-up?
