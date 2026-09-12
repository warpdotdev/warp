# Renaming Conversations — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/8642
Prior art: https://github.com/warpdotdev/warp/pull/9646
Figma: none provided. The prior PR review includes screenshots of the existing tab rename interaction, which is the intended interaction model.
## Summary
Users can set, edit, and reset a custom title for an agent conversation. The custom conversation title behaves like Warp's existing custom tab title flow: users can double-click the visible conversation title to rename it inline, use menu actions to rename or reset it, and rely on the name persisting after the tab is closed and the conversation is reopened later.
## Problem
Conversation titles are currently derived from the agent-generated task description, the user's prompt, or a fallback title. Users can rename the tab that happens to contain a conversation, but that tab name is not the conversation name, does not survive as the searchable title after the tab closes, and does not help identify the same conversation from the conversation list later. Issue #8642 reports that a previous conversation-pill rename affordance disappeared and users cannot find another way to rename conversations.
## Goals
- Provide a per-conversation custom title that survives tab closure, restart, and later restore on the same machine.
- Match existing tab rename behavior where possible: double-click-to-rename, inline editor, Enter/blur to save, Escape to cancel, and an explicit reset menu item.
- Make the custom title the displayed and searchable conversation title wherever local conversation titles are shown.
- Keep tab names and conversation names independent so users can still rename a tab without mutating the underlying conversation.
## Non-goals
- Cloud sync of custom conversation titles. This iteration stores custom titles locally only.
- Renaming cloud-only or task-only ambient agent rows that do not have local conversation data.
- Adding a new global keyboard shortcut, slash command, or telemetry event for conversation rename.
- Changing the agent-generated title, task description, first-prompt fallback, or latest-prompt setting beyond letting a custom title override those derived values.
- Migrating existing custom tab names into conversation titles.
## Behavior
1. A conversation with no custom title displays exactly the same title it displays today. The existing derived priority remains unchanged underneath the feature: agent-generated task description, then the initial user prompt, then any existing fallback title.
2. When a user sets a custom conversation title, that custom title becomes the displayed conversation title for that conversation on this device.
3. The custom title appears everywhere Warp shows the local conversation title, including:
   - The primary conversation title line in vertical tabs.
   - Conversation rows in the conversation list panel.
   - Conversation results and fork-current-conversation rows in command palette conversation search.
   - The OS window title or tab title when that title is derived from the active conversation rather than from a manually renamed tab.
4. A custom conversation title does not replace or mutate a custom tab title. If the user has manually renamed the tab, the tab-level name continues to win for tab-level chrome. The conversation card, conversation list, and conversation search still use the conversation title.
5. The existing `appearance.vertical_tabs.use_latest_prompt_as_title` setting continues to apply only when no custom conversation title exists. If a custom conversation title exists, it wins regardless of whether that setting would otherwise choose the latest prompt.
6. In vertical tabs, when the primary line is an agent conversation title, double-clicking that title starts inline rename for the conversation. This matches tab rename: the title becomes a single-line inline text editor, the current title is selected, Enter saves, blur saves, and Escape cancels.
7. In vertical tabs, right-clicking an agent conversation card opens the existing context menu with a `Rename conversation` item. Selecting it starts the same inline rename state as double-clicking the title.
8. If the conversation already has a custom title, the vertical-tabs context menu also shows `Reset conversation name`. Selecting it immediately clears the custom title and returns the visible title to the derived title without opening the editor.
9. In the conversation list panel, the overflow menu for a local conversation includes `Rename conversation`. Selecting it starts inline rename in that row, with the current title selected. Enter saves, blur saves, and Escape cancels.
10. If a conversation-list row already has a custom title, its overflow menu also includes `Reset conversation name`. Selecting it clears the custom title immediately and updates the row to the derived title.
11. Starting rename closes any open conversation context menu or overflow menu for that same item so the inline editor is not obscured.
12. While an inline rename editor is active, normal click-to-open behavior for that title or row is suppressed. Keyboard focus is in the editor until the user saves or cancels.
13. Saving a changed non-empty value stores the edited value as the custom conversation title. Leading and trailing whitespace are trimmed before saving. Internal whitespace, punctuation, non-Latin text, and emoji are preserved.
14. Saving an empty or whitespace-only value clears the custom conversation title. This produces the same end state as choosing `Reset conversation name`.
15. If the user saves without changing the editor contents, Warp exits rename mode and leaves the current custom-title state unchanged.
16. Custom titles have a maximum length of 200 Unicode scalar values. Input beyond that limit is ignored or truncated without showing a blocking error.
17. A successful save or reset updates all visible local surfaces immediately. No restart, tab reopen, or manual refresh is required.
18. Custom titles persist locally across app restart. Reopening the conversation from the conversation list shows the custom title.
19. Renaming a past conversation from the conversation list works when the conversation has local data but is not currently open in a tab. The row updates immediately, and opening that conversation later shows the same custom title.
20. Conversation search matches the custom title. If a custom title is reset, search falls back to matching the derived title and initial query as it did before.
21. Renaming is allowed while the conversation is in progress. Later streaming updates or task-description updates do not overwrite the custom title. Resetting the custom title returns the title to the latest derived title at that moment.
22. Forking a conversation does not copy the source conversation's custom title. The fork starts with no custom title and displays its own derived fork title until the user renames it.
23. Shared-session viewer mode does not expose conversation rename or reset actions. A viewer sees the title supplied by the shared conversation data.
24. Cloud-only conversation rows and ambient-agent task rows without local conversation data cannot be renamed in this iteration. If they appear in a menu that otherwise includes conversation management actions, `Rename conversation` is disabled with explanatory tooltip text rather than failing after selection.
25. The feature must not create duplicate naming concepts in the UI. Menu labels use `conversation` for conversation title actions and `tab` or `pane` for existing tab/pane actions so users can tell which object they are renaming.
26. The existing tab rename and pane rename flows keep their current behavior and labels. This feature must not regress double-click tab rename, `Rename tab`, `Reset tab name`, pane rename, or pane reset.
