# WARPER-012: copy code review comments to clipboard

## Summary

Add a Copy to Clipboard action for pending code review comments in the diff review surface. The action gives the user a disposable handoff payload they can paste into an external agent, then clears the copied comments from the review UI.

## Problem

The current Send to Agent action assumes Warper can route comments to an in-app agent or CLI agent. Users sometimes need to hand the same review data to an external agent manually, without depending on an active terminal destination.

## Figma

Figma: none provided.

## Behavior

1. When the comment list contains at least one non-outdated review comment, the user can copy the active review comments to the system clipboard from the same footer area that contains Send to Agent.

2. The copy action is independent of agent availability. It remains available when AI is disabled, when no terminal can receive comments, or when all terminals are busy, as long as there is at least one non-outdated comment to copy.

3. When there are no non-outdated comments, the copy action is disabled or unavailable. Outdated comments are not copied.

4. Activating Copy to Clipboard writes a plain-text review packet to the clipboard. The packet is neutral data, not an agent-specific prompt. It must include:
   - Each copied comment body.
   - The file path for each file or line comment.
   - The line or range when the comment is attached to a line, removed line, or collapsed hunk.
   - A clear marker for general comments.
   - Relevant diff context for the copied comments when available.

5. The copied text must not include marketing copy, UI labels, hidden internal IDs, timestamps, debug fields, or phrasing that assumes a specific destination agent.

6. The copied text must preserve user-authored comment text accurately. Markdown comments may be represented as readable plain markdown, but punctuation must not gain extra escaping that the user did not write.

7. After the clipboard write is accepted by the app, the copied comments are cleared from the review UI. This mirrors the disposable handoff model: once the comments are copied for external handling, they should not remain as pending review work in Warper.

8. Clearing after copy removes only the comments that were eligible for copying. It must not discard unrelated file changes, diff selections, editor content, unsaved edits, or unrelated comments owned by another repository or pane.

9. After a successful copy, Warper shows a short confirmation message that makes both effects explicit: comments were copied and cleared.

10. If the clipboard write fails and the app can detect the failure, Warper does not clear comments and shows an error. If the platform cannot report clipboard success or failure synchronously, Warper may treat the user action as accepted, but the user-facing behavior should avoid implying stronger delivery guarantees than the platform provides.

11. Cancel remains distinct from copy. Cancel clears comments without writing anything to the clipboard.

12. Send to Agent remains distinct from copy. Send routes comments to the selected agent destination and clears comments only after successful submission; Copy writes to the clipboard and clears comments without requiring an agent destination.

13. Multiple code review panes or repositories remain isolated. Copying comments from one review surface does not copy or clear comments from another active review surface.

14. The button label, tooltip, and confirmation text must make the destructive part understandable before or immediately after activation.

15. The feature is only required on local desktop review surfaces where code review comments and local diffs are available. Browser/WASM surfaces that cannot provide local diff context are not required to expose this action.
