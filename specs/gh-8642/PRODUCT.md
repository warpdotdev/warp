# Conversation renaming
## Summary
Warp should let users manually rename the currently selected AI conversation with a `/rename-conversation` slash command. The rename should be server-confirmed before any local title changes, update the conversation's canonical title, refresh every surface that displays that title, and remain sticky even if generated title logic runs later.
## Problem
Auto-generated AI conversation titles can be generic, duplicated, or misleading after a conversation evolves. Users need a small, reliable way to give an active conversation a human-readable name that is reflected everywhere Warp shows that conversation.
## Goals / Non-goals
Goals:
- Add a first-pass `/rename-conversation <title>` command for the currently selected conversation.
- Treat the renamed value as the conversation's canonical title, not as a separate display-only alias.
- Keep all title-bearing surfaces in sync after the server confirms the rename.
- Ensure a manually renamed title wins over future generated titles until the user renames the conversation again.
- Use the same conversation permission model as existing conversation endpoints, requiring edit access for rename.
Non-goals:
- No sidebar, details-panel, command-palette, or inline edit UI in this first pass.
- No conversation-id argument or ability to rename historical conversations that are not currently selected.
- No reset-to-generated-title behavior in this first pass.
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
8. On server success, Warp updates the conversation's canonical title locally and every open local surface backed by that title refreshes to show the new title. This includes pane title, tab title, vertical tabs summaries, conversation list/search results, conversation details, and the conversation management panel.
9. On server failure, permission denial, missing server conversation identity, timeout, or offline/network failure, Warp shows an error toast and the local conversation title remains unchanged. The user can retry the command.
10. If there is no active selected conversation, Warp shows an error toast and does not send a server request.
11. If the active conversation exists only locally and cannot be mapped to a server conversation yet, Warp shows an error toast and does not change the title. If the same conversation later receives a server identity, the user can retry.
12. Renaming is allowed while an agent response is in progress. If the server accepts the rename while a response is still streaming, the renamed title remains the conversation title after that response finishes.
13. A renamed title remains sticky across future generated-title writes, follow-ups, auto-resume flows, run completion, cloud reloads, and app restarts. Generated title logic must not overwrite the user-facing conversation title.
14. If the user runs `/rename-conversation` again on the same conversation, the new server-confirmed title replaces the previous title and becomes the sticky title.
15. Server-backed surfaces that store or display the same conversation title should converge on the renamed title after the server rename succeeds. This includes conversation metadata surfaces and agent run/task management surfaces associated with the conversation.
16. If the same conversation is visible in another client or session, that other client is not required to update in real time, but after refresh, restore, or refetch it should show the renamed title.
17. Search and filtering that use conversation titles should match the renamed title after success. The original initial query remains available wherever initial-query search already exists.
18. Existing permissions apply. The server checks the same conversation object permissions used by other conversation endpoints, but the required action is edit access. A user who can view but not edit a shared conversation cannot rename it; Warp reports the failure and leaves the local title unchanged.
19. If the conversation has no cloud-stored transcript/task data, the server still accepts the rename after metadata permission checks pass. In that case the server updates metadata and task title surfaces and skips only the cloud `ConversationData` update.
20. Forking or handing off a renamed conversation uses the renamed title as the source conversation's title unless a more specific fork or handoff title override is supplied.
