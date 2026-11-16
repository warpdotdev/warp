# PRODUCT: Warp TUI Conversation Resume and Restoration (CODE-1820)

## Summary

The Warp TUI prints a resumable server conversation token when the user exits with a non-empty selected conversation and accepts that token through `--resume` on a later launch. Resuming restores the selected Oz/Warp conversation into the TUI's single conversation surface, including its transcript and conversation-derived command history, so the next prompt continues the same conversation.

## Figma

Figma: none provided.

## Goals

- Make a TUI conversation easy to continue after exiting and relaunching the TUI.
- Give locally available and server-fetched conversations the same user-visible restoration behavior.
- Define restoration behavior that the upcoming inline TUI conversation list can invoke without changing the resulting transcript or continuation semantics.

## Non-goals

- Restoring agent orchestration subscriptions, child-agent event delivery, or other orchestration runtime state.
- Restoring or continuing conversations created by Claude Code, Gemini, Codex, or any non-Oz harness. The Warp TUI supports only Oz/Warp conversations.
- Restoring the previous TUI process, PTY, or shell session. Conversation-derived command blocks are restored as transcript history, but no command is restarted and no prior shell process is recreated.
- Adding the inline conversation list itself.
- Supporting multiple simultaneous TUI conversation surfaces.

## Behavior

### Starting and resuming

1. Launching the TUI without `--resume` opens its normal empty state backed by an eagerly created conversation; the first prompt continues that conversation.

2. Launching the TUI with `--resume <server-conversation-token>` attempts to restore the Oz/Warp conversation identified by that token.

3. The resume token is the server conversation token, regardless of whether the conversation transcript is stored in the cloud. Users never need to provide or distinguish a local conversation ID.

4. A malformed token is rejected with a clear error. The TUI does not open an empty conversation or reinterpret the malformed value as another identifier.

5. Resume follows normal authentication behavior. If the user must log in, restoration begins after login succeeds; the token remains pending throughout login without requiring the user to enter it again.

6. While restoration is in progress, the TUI shows a visible loading state for the requested conversation. It does not briefly show an interactive new-conversation state that could accept a prompt for the wrong conversation.

7. If the requested conversation is available locally, it can be restored without network access.

8. If the requested conversation is not available locally, the TUI fetches it from the server. Local and server-fetched restoration produce the same transcript and continuation behavior.

### Restored transcript

9. A successful restore replaces the TUI's sole conversation surface with the requested conversation.

10. The restored transcript preserves the original visible ordering of:
    - User prompts.
    - Agent text and reasoning supported by the TUI.
    - Tool calls and their recorded results.
    - Conversation-derived command blocks, including recorded commands, output, and completion state.

11. Content hidden by the conversation remains hidden after restoration. Internal task types that are normally omitted from the blocklist remain omitted.

12. Restored tool calls show their recorded completed, failed, cancelled, or blocked state rather than appearing pending or unknown.

13. Restoring command history is display-only. No restored command is executed, no command process is restarted, and restoring a transcript does not modify the current filesystem by itself.

14. The restored transcript does not contain duplicate exchanges or command blocks.

15. After restoration succeeds, the transcript is positioned at its newest content, the input is focused, and the user can immediately type a follow-up.

### Continuing the conversation

16. The first prompt submitted after restoration continues the same server conversation identified by the resume token. Restoration does not fork the conversation or start a new server lineage.

17. Resuming alone does not send a prompt, restart work, or otherwise mutate the conversation.

18. A completed, cancelled, or failed conversation can still receive a new follow-up after restoration, subject to the same server-side permissions and request behavior as the GUI.

19. If the upcoming inline conversation list invokes restoration while another conversation is displayed, the current transcript remains visible until the requested conversation has loaded successfully. The surface is replaced only after loading succeeds.

20. If inline restoration fails, the existing selected conversation and transcript remain intact.

### Errors and cancellation

21. Restoration failures are visible and do not expose an interactive empty conversation. Malformed tokens are rejected before launch, non-Oz conversations receive an explicit unsupported-harness message, and other loading failures use the same generic failure semantics as the GUI conversation loader.

22. A restoration error never silently falls back to a new conversation and never sends a prompt.

23. A non-Oz token produces an explicit message that the Warp TUI only supports Oz/Warp conversations.

24. While startup restoration is loading, Escape or Ctrl-C cancels the restore and continues into the provisional new TUI session as though `--resume` had not been supplied.
    - The loading screen shows `Esc or Ctrl-C to cancel and start a new session`.
    - The provisional conversation becomes interactive without requiring a restart.
    - A late loader result is ignored and cannot replace the new session.
    - The cancelled target does not become selected and does not produce an exit resume hint.

### Exit resume hint

25. When the TUI exits successfully with a selected conversation that contains at least one exchange, it prints a single resume instruction after leaving the full-screen TUI and restoring the host terminal.

26. The instruction contains the selected conversation's server token in the form:
    `warp-tui --resume <server-conversation-token>`

27. The hint is based only on the selected conversation. The TUI does not fall back to another active or recently streamed conversation.

28. If no conversation is selected, or the selected conversation is empty because the user never submitted a prompt, no resume hint is printed.

29. If the selected conversation has not received a server token because initialization did not complete, no resume hint is printed rather than printing a local ID or an unusable command.

30. Internal worker processes, malformed startup invocations, and TUI launches that terminate with an error do not print a success-style resume hint.

31. Abrupt process termination, such as a forced kill, is not guaranteed to print the hint. Conversations remain subject to the existing persistence guarantees independently of whether the message can be displayed.
