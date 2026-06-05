# Conversation renaming
## Summary
Warp should let users manually rename the currently selected AI conversation with a `/rename-conversation` slash command. The rename should be server-confirmed before any local title changes, persist across restarts and cloud reloads, and remain sticky even if generated title logic runs later.
## Problem
Auto-generated AI conversation titles can be generic, duplicated, or misleading after a conversation evolves. Users need a small, reliable way to give an active conversation a human-readable name that is reflected everywhere Warp shows that conversation.
## Goals / Non-goals
Goals:
- Add a first-pass `/rename-conversation <title>` command for the currently selected conversation.
- Keep the implementation consistent with existing slash-command and toast patterns.
- Ensure a manual title wins over future generated titles until the user renames the conversation again.
- Use the same conversation permission model as existing conversation endpoints, requiring edit access for rename.
Non-goals:
- No sidebar, details-panel, command-palette, or inline edit UI in this first pass.
- No conversation-id argument or ability to rename historical conversations that are not currently selected.
- No reset-to-generated-title behavior in this first pass.
- No rewriting historical transcript/task bytes just to update the display title.
## Figma
Figma: none provided. This first pass reuses existing slash-command and toast patterns.
## Behavior
1. When the user has an active AI conversation selected, they can run `/rename-conversation <title>` from the input surfaces that support active-conversation slash commands.
2. `/rename-conversation` operates only on the currently selected active conversation. It never renames an arbitrary conversation by id, token, title match, or search result.
3. The command requires a title argument. If the user runs `/rename-conversation` with no argument, or with an argument that becomes empty after trimming leading and trailing whitespace, Warp shows an error toast and does not send a server request.
4. The title stored for the conversation is the argument after trimming leading and trailing whitespace. Internal repeated whitespace is preserved.
5. Titles have a maximum accepted length of 500 Unicode scalar values. If the trimmed title is longer than the limit, Warp shows an error toast and does not send a server request.
6. Warp must not update the local conversation title optimistically. After the command is accepted, the currently visible title remains unchanged until the server rename request succeeds.
7. While the server rename request is in flight, the command uses existing non-blocking slash-command behavior. The user can continue using Warp; the conversation title remains the previous title until success.
8. On server success, Warp updates the local conversation title and every local surface backed by the conversation title refreshes to show the new title. This includes the active conversation header, conversation list/search results, vertical tabs summaries, details surfaces, and any restored local conversation metadata.
9. On server failure, permission denial, missing server conversation identity, timeout, or offline/network failure, Warp shows an error toast and the local conversation title remains unchanged. The user can retry the command.
10. If there is no active selected conversation, Warp shows an error toast and does not send a server request.
11. If the active conversation exists only locally and cannot be mapped to a server conversation yet, Warp shows an error toast and does not change the title. If the same conversation later receives a server identity, the user can retry.
12. Renaming is allowed while an agent response is in progress. If the server accepts the rename while a response is still streaming, the manual title remains the conversation title after that response finishes.
13. A manual title remains sticky across future generated-title writes, follow-ups, auto-resume flows, run completion, cloud reloads, and app restarts. Generated title logic must not overwrite the user-facing conversation title.
14. If the user runs `/rename-conversation` again on the same conversation, the new server-confirmed manual title replaces the previous manual title and becomes the sticky title.
15. Server-backed surfaces that store or display the same conversation title should converge on the manual title after the server rename succeeds. This includes conversation metadata surfaces and agent run/task management surfaces associated with the conversation.
16. If the same conversation is visible in another client or session, that other client is not required to update in real time, but after refresh, restore, or refetch it should show the manual title.
17. Search and filtering that use conversation titles should match the new manual title after success. The original initial query remains available wherever initial-query search already exists.
18. Existing permissions apply. The server checks the same conversation object permissions used by other conversation endpoints, but the required action is edit access. A user who can view but not edit a shared conversation cannot rename it; Warp reports the failure and leaves the local title unchanged.
19. Transcript/task descriptions that are part of the historical conversation content are not themselves renamed in this first pass. The manual title is the display title for conversation-title surfaces; transcript content remains the original recorded content.
