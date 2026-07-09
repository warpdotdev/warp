# Embedded Code Review Comments

## Summary
Embedded code review comments render directly inside the code review diff at the line they reference. Users can create, read, edit, update, remove, and cancel comments inline without switching to a separate bottom-panel-only workflow, and inline comments should visually behave like part of the diff row rather than like floating overlays.

## Problem
The previous inline review experience used gutter icons and floating composers that could obscure code, duplicate saved-comment affordances, and shift between different visual treatments when moving from draft to saved state. The desired experience is closer to GitHub's inline review model: the comment appears below the reviewed line, the surrounding diff background continues through the comment area, and saved comments use the same stable comment surface as edits.

## Figma
Figma: none provided.

## Goals
- Make line-targeted review comments visible in context, directly below their referenced diff line.
- Keep the inline comment surface stable when a comment transitions between draft, saved, and editing states.
- Preserve the existing non-embedded / floating composer behavior when embedded comments are disabled.
- Keep file-level, general, and outdated comments out of the inline diff surface.

## Non-goals
- Replacing the bottom review comments panel.
- Changing how review comments are submitted to agents or GitHub.
- Supporting threaded inline conversations beyond the existing flattened comment body behavior.

## Behavior
1. When embedded code review comments are enabled, creating a comment on a diff line opens an inline comment editor directly below that line.

2. The inline editor is anchored to the reviewed line, not to the user's current cursor position or scroll position. Vertical scrolling moves the editor with the surrounding diff content.

3. The inline editor appears below the reviewed line. It must not cover the text of the reviewed line.

4. The diff background for the reviewed hunk continues behind the inline comment editor. Added-line comments continue the added-line background; removed-line comments continue the removed-line background; replacement hunks preserve the correct added or removed side color for the line being commented on.

5. The inline comment editor is horizontally stable relative to the visible editor viewport. Horizontal code scrolling must not clip the comment editor or cause the card to move off-screen with long code lines.

6. Inline comment cards use a consistent shell in draft, saved, and editing states: the same max width, rounded border, background, body padding, footer border, footer padding, and bottom-aligned actions.

7. A new draft comment has an editable body and footer actions for canceling or saving the comment. The primary save action is disabled while the draft body is empty.

8. Saving a new draft converts the same inline comment surface into a saved comment. The transition should not visibly flicker, jump height, swap to a narrower card for a frame, or briefly show an empty editor.

9. Saved comments show the rendered comment body and a footer with lightweight metadata, including relative update time. If a comment was imported from GitHub, the saved/footer metadata includes a GitHub indicator.

10. Saved comments show Edit and Remove actions in the footer. Edit appears to the left of Remove.

11. Selecting Edit on a saved inline comment changes that same comment surface into an editable state. It must not replace the card with a separate editor view or cause a visible size jump.

12. Editing an existing comment shows the saved body as the editable draft, uses an Update primary action, and includes Cancel and Remove actions.

13. Saving an edit updates the same inline comment surface back to saved mode with the updated body.

14. Canceling an edit restores the saved content and returns the same inline comment surface to saved mode.

15. Canceling a new unsaved draft removes that draft from the inline diff surface.

16. Removing a saved inline comment deletes the comment from the review comment set and removes the inline card from the diff.

17. Multiple saved comments on distinct lines render as independent inline cards at their own lines.

18. Comments on the same current line stack without overlapping each other or the surrounding code.

19. Removed-line comments are anchored to the specific removed-line slot they reference, not just to the current line number of the hunk.

20. A removed-line comment and a current-line comment with the same displayed line number are treated as different anchors.

21. When the embedded comments feature is disabled, opening a line comment uses the existing floating comment composer behavior. Lines below the commented line must not be pushed down by an inline block in this mode.

22. When embedded comments are enabled, the old saved-comment gutter icon is hidden for lines that already have inline cards. The inline card itself is the saved-comment affordance.

23. When an inline draft or edit is active on a line, the gutter should not show duplicate comment icons or action buttons for that same inline comment row.

24. Gutter add-comment affordances remain available for changed lines that do not currently have an inline draft or saved inline comment occupying their comment affordance.

25. The inline comment block should not render a duplicate line number for the reserved comment row. Only actual code/diff lines show line numbers.

26. Inline comment cards do not render the diff snippet that bottom-panel comment cards include. The line context is already visible in the surrounding diff.

27. File-level comments, general/diffset comments, and outdated line comments do not render as inline cards in the diff. They remain visible through the existing review comment surfaces.

28. Switching diff modes or refreshing the review comment batch updates inline comment positions to the latest valid line anchors. Comments that can no longer be anchored inline should not leave stale inline cards behind.

29. Inline comments preserve keyboard and focus expectations: opening a draft or edit focuses the editable comment body; saved comments are selectable but not editable; escape/cancel behavior should not discard non-empty drafts unexpectedly.

30. The review experience must remain usable with long code lines, horizontal scrolling, expanded deletion hunks, replacement hunks, and multiple comments in one file.
