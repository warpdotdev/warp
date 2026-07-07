# Cloud-to-cloud handoff resumable completed conversations
## Summary
Completed cloud agent conversations should be resumable in cloud mode when cloud-to-cloud handoff is available. Opening a completed cloud task should restore a Cloud Mode pane seeded with the prior conversation history, preserving the familiar completed transcript/tombstone experience while allowing the user to submit a follow-up prompt that starts a new cloud execution in the same conversation.
## Problem
Before cloud-to-cloud handoff, a cloud task with no active execution was permanently complete from the client’s perspective, so the transcript viewer and “Continue locally” affordance were sufficient. With multi-execution cloud runs, the same visual state can now be a pause between executions. Users should not hit a dead-end toast when the product offers a cloud Continue action.
## Figma
Figma: none provided. The current failure state is represented by the screenshot attached to the request.
## Behavior
1. When a user opens a cloud agent conversation that has no active execution and cloud-to-cloud continuation is available, Warp opens a Cloud Mode pane seeded with the completed conversation history instead of starting a new blank task.
2. The restored Cloud Mode pane preserves the existing completed-conversation presentation:
   - The prior conversation output remains visible.
   - The completed task tombstone remains visible at the end of the transcript.
   - Existing metadata, artifacts, error status, runtime, credits, source, skill, and working-directory details remain available wherever they are already shown.
3. For a resumable cloud task, the tombstone shows a primary `Continue` action for continuing in cloud mode.
4. The existing `Continue locally` action remains available where it is supported today. Adding cloud continuation must not remove the local fork path.
5. If the task cannot be continued in cloud mode, Warp does not show a cloud `Continue` action. Users should not be offered an action that can only produce a generic “couldn't continue” failure.
6. Clicking cloud `Continue` does not immediately create a new execution. It prepares the same Cloud Mode pane for a follow-up prompt and focuses the existing terminal input.
7. While the pane is waiting for a follow-up prompt, the user can type, edit, or abandon the prompt using the normal terminal input behavior.
8. Submitting a non-empty follow-up prompt starts a new execution for the same cloud task/run, not a new user-visible conversation.
9. Submitting an empty follow-up prompt does not start a new execution. The pane stays usable and focused so the user can type a real prompt or leave the view.
10. After the follow-up prompt is submitted, the restored pane transitions into Cloud Mode setup/progress UI in the same pane.
11. During follow-up setup, the user sees their submitted prompt represented optimistically so the pane does not appear to ignore the input while the new cloud execution is starting.
12. When the new execution starts, Warp attaches to the new cloud session in the same pane. It does not open a separate tab, split, or replacement conversation unless the user separately chooses another navigation action.
13. The same logical conversation/run identity is preserved across the original transcript and every follow-up execution.
14. New output from the follow-up appears after the existing transcript content in chronological order.
15. Returning to the conversation list, agent management view, details panel, or another navigation surface should focus the already-open pane for this task while it is open.
16. If a follow-up execution finishes, the pane returns to a completed-between-executions state and can offer cloud `Continue` again when the task is still resumable.
17. Repeated follow-ups are allowed. A second or later follow-up should behave the same as the first: prompt input, setup/progress, same-pane session attachment, and preserved conversation identity.
18. A failed, blocked, cancelled, timed-out, unauthorized, or quota-limited follow-up attempt surfaces through the same user-facing Cloud Mode error/auth/capacity/credits states used by cloud task startup.
19. If the follow-up request fails before the prompt is accepted, the user’s prompt is restored in the input so they can edit or retry.
20. If the follow-up request is accepted but the new execution fails before a session is available, the pane shows the appropriate failure state and does not silently discard the completed conversation history.
21. Closing the pane while a follow-up execution is starting stops only the local viewing/waiting experience. It must not imply that the remote cloud run is cancelled unless the user explicitly invokes a cancel action.
22. Opening a completed local agent conversation still uses the existing local transcript behavior. Cloud continuation is only for cloud/ambient agent conversations that are resumable in cloud mode.
23. Opening a non-owned or otherwise non-resumable cloud transcript still works as a read-only transcript. It must not incorrectly expose cloud continuation.
24. If the cloud-to-cloud continuation feature is unavailable or disabled, completed cloud conversations continue to use the existing transcript/tombstone behavior.
25. Existing transcript share/open behavior should continue to work while the view is idle between cloud executions.
26. Keyboard focus should move to the prompt input after cloud `Continue` is clicked. Users should not need to click the input manually before typing the follow-up prompt.
27. The UI should avoid stacking confusing duplicate tombstones for the same between-executions state. After a follow-up starts, the old tombstone should not remain as an actionable end marker above the active setup/progress state.
28. The generic “Couldn't continue this cloud task.” toast is only acceptable for genuinely unexpected inconsistencies. It should not be part of the normal completed-cloud-task continuation flow.
