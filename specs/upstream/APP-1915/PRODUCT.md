# APP-1915: Copy URL / Copy path in AI response right-click context menu

## Summary

When a user right-clicks a hyperlink rendered inside an AI response, add a "Copy URL" (for web URLs) or "Copy path" (for file paths) item to the existing AI block context menu, grouped with the other Copy items. The full AI block context menu must still be shown — the link-specific item is an addition, not a replacement.

## Problem

AI responses often contain links (URLs and file paths). Today there is no quick way to copy a link target from the context menu; users have to manually select the link text. Terminal grid links already offer "Copy URL" / "Copy path" on right-click, so AI responses are an outlier.

A previous change on `oz-agent/copy-url-in-ai-response-context-menu` added the affordance but replaced the entire AI block right-click context menu with a one-item "Copy URL" menu, regressing every other right-click action (Share session, Copy, Copy prompt, Copy output as Markdown, Save as prompt, Share conversation, Fork…, Rewind…, Copy debugging link/ID, Split pane…, Close pane) whenever the cursor happened to be over a link. That is the bug this spec addresses.

## Non-goals

- Adding "Show in Finder", "Open in Warp", or "Open in editor" items for file-path links in AI responses. Scope is limited to copying the target.
- Changing the terminal grid link context menu.
- Adding a separate slimmed-down link-only menu (the earlier attempt). The existing AI block menu is kept intact.
- Changing how hyperlinks are detected or rendered inside AI responses.

## Figma

Figma: none provided. The existing AI block context menu (see screenshots attached to APP-1915) is the baseline; the only visible change is an additional "Copy URL" or "Copy path" item inserted next to the other Copy items.

## Behavior

1. Right-clicking a URL hyperlink inside an AI response shows the full existing AI block context menu (Share session, Copy, Copy prompt, Copy output as Markdown, Save as prompt, Share conversation, Copy conversation text, Fork…, Rewind…, Copy debugging link, Copy conversation ID, Split pane…, Close pane) with no items removed and no other items reordered.

2. When the cursor is over a URL hyperlink, a "Copy URL" item is inserted into the menu immediately after "Copy output as Markdown" and before any conditional "Copy command" / "Copy git branch" items. Selecting it writes the hovered URL string verbatim to the clipboard (the URL target, not the displayed link text).

3. When the cursor is over a file-path hyperlink, a "Copy path" item is inserted in the same position as "Copy URL" (immediately after "Copy output as Markdown"). Selecting it writes the absolute path of the hovered file to the clipboard.

4. "Copy path" is only available on builds that have the `local_fs` feature enabled, matching the existing file-path link behavior elsewhere in the app. On builds without `local_fs`, no "Copy path" item appears and the rest of the menu is unchanged.

5. At most one link-specific item is inserted per menu: "Copy URL" xor "Copy path", never both, and never duplicated for overlapping link regions.

6. The order of Copy items within the AI block menu is stable: Copy → Copy prompt → Copy output as Markdown → (Copy URL or Copy path, when on a link) → Copy command (when applicable) → Copy git branch (when applicable). Non-Copy items retain their existing relative order.

7. Right-clicking anywhere inside an AI response where the cursor is not on a hyperlink shows the existing AI block context menu unchanged — no link-specific items, no reorderings, no omissions.

8. Right-clicking inside an AI response while a text selection is active shows the existing selection-oriented menu (Copy, Insert into input, optionally Attach as agent mode context). No "Copy URL" or "Copy path" item is added in this case, even if the selection overlaps a link — the user's primary intent is the selection.

9. Right-clicking a link in the terminal grid (outside AI responses) is unchanged: it continues to show the existing grid link context menu (Copy URL / Copy path / Show in Finder / Open in Warp / Open in editor).

10. Right-click never crashes or panics when there is no hovered link at the time the menu is requested; the link-specific item is simply omitted.

11. The link-specific item is computed from the hover state at the moment the menu is opened. If the hovered link changes or disappears while the menu is open, the already-shown menu is not mutated; the next right-click recomputes from the new hover state.
