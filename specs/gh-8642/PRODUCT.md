# Conversation renaming
## Summary
Warp should let users manually rename the currently selected AI conversation with a `/rename-conversation` slash command. For server-backed conversations, Warp should apply the rename locally immediately, refresh every local surface that displays that title, and asynchronously sync the rename to the server. If server sync fails, Warp should show an error toast and revert that failed local rename.
## Problem
Auto-generated AI conversation titles can be generic, duplicated, or misleading after a conversation evolves. Users need a small, reliable way to give an active conversation a human-readable name that is reflected everywhere Warp shows that conversation.
## Goals / Non-goals
Goals:
- Add a first-pass `/rename-conversation <title>` command for the currently selected conversation.
- Treat the renamed value as the conversation's canonical title, not as a separate display-only alias.
- Keep all title-bearing local surfaces in sync immediately after the local rename is accepted.
- Revert the local rename if the server rejects or fails to persist it.
- Ensure a manually renamed title wins over future generated titles until the user renames the conversation again.
- Use the same conversation permission model as existing conversation endpoints for server-side persistence, requiring edit access for rename.
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
6. After validation succeeds for a server-backed active conversation, Warp immediately updates the local conversation's canonical title before waiting for the server rename request to complete.
7. The local update refreshes every open local surface backed by that title. This includes pane title, tab title, vertical tabs summaries, conversation list/search results, conversation details, and the conversation management panel.
8. Warp asynchronously sends the server rename request after applying the local title. The command uses existing non-blocking slash-command behavior while that request is in flight, and the user can continue using Warp.
9. On server success, Warp treats the rename as persisted and shows a success toast: `Conversation renamed to <title>`. If the server returns a normalized title different from the locally applied trimmed title, Warp updates the local canonical title to that returned title and refreshes title-bearing surfaces again.
10. On server failure, permission denial, or offline/network failure, Warp shows an error toast and reverts the local title to the title that was visible before that rename request was applied.
11. If a rename is already in progress for the selected conversation, another `/rename-conversation` request for that conversation is rejected early. Warp shows an error toast, does not change the local title, and does not send another server request.
12. If there is no active selected conversation, Warp shows an error toast, does not change any local title, and does not send a server request.
13. If the active conversation exists only locally and cannot be mapped to a server conversation yet, Warp shows an error toast that tells the user to send another message and retry. Warp does not change the title in this first pass. If the same conversation later receives a server identity, the user can retry.
14. Renaming is allowed while an agent response is in progress as long as the conversation already has a server identity and no rename is already pending for that conversation. If the server accepts the rename while a response is still streaming, the renamed title remains the conversation title after that response finishes.
15. A locally accepted title remains sticky across future generated-title writes, follow-ups, auto-resume flows, run completion, cloud reloads, and app restarts unless the server sync for that rename fails and Warp reverts it. Generated title logic must not overwrite the user-facing conversation title while server sync is in flight.
16. After a rename succeeds or fails and its in-flight state clears, the user can run `/rename-conversation` again on the same conversation.
17. Server-backed surfaces that store or display the same conversation title should converge on the renamed title after the server rename succeeds. This includes conversation metadata surfaces and agent run/task management surfaces associated with the conversation.
18. If the same conversation is visible in another client or session, that other client is not required to update in real time, but after refresh, restore, or refetch it should show the renamed title once server sync has succeeded.
19. Search and filtering that use conversation titles should match the renamed title immediately after the local update. If the server sync fails and Warp reverts the title, search and filtering should match the reverted title. The original initial query remains available wherever initial-query search already exists.
20. Existing permissions apply on the server. The server checks the same conversation object permissions used by other conversation endpoints, but the required action is edit access. A user who can view but not edit a shared conversation sees the local title update first, then receives a server-sync failure toast and Warp reverts the local title.
21. If the conversation has no cloud-stored transcript/task data, the server still accepts the rename after metadata permission checks pass. In that case the server updates metadata and task title surfaces and skips only the cloud `ConversationData` update.
22. Forking or handing off a renamed conversation uses the current local conversation title as the source conversation's title unless a more specific fork or handoff title override is supplied. If server sync fails before the fork or handoff starts, the reverted title is used.
