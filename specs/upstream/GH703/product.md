# PRODUCT.md — Linear deeplink must not silently auto-submit prompts into the agent

**GitHub Issue:** [warpdotdev/warp-external#703](https://github.com/warpdotdev/warp-external/issues/703)
**Figma:** none provided

## Summary

`warp://linear/work?prompt=<PROMPT>` currently drops the URI-supplied prompt straight into the user's active agent conversation and, when the user is already in fullscreen agent view, submits it to the LLM with no visible confirmation. The feature must instead always treat a Linear deeplink prompt as untrusted input: it should populate the agent input buffer, make it visually obvious where that prompt came from, and require an explicit user gesture before anything is sent to the model.

## Problem

The Linear "work on issue" deeplink is a trusted URI surface: the handler accepts any `prompt=` query value and forwards it verbatim into `enter_agent_view_for_new_conversation(prompt, LinearDeepLink, ...)`. Inside `try_enter_agent_view`, an auto-submit branch fires when the terminal was already in fullscreen agent view, even though `LinearDeepLink` is intentionally excluded from `should_autotrigger_request`'s allowlist. Combined with any of Warp's `warp://` dispatch primitives (macOS URL handler, Linux D-Bus, Windows named pipe, third-party openers), an attacker-controlled URL can silently inject a prompt — and, transitively, trigger auto-executed AI tools — in the currently authenticated user's session with no UI indication.

Users of Warp's AI features need to be able to trust that nothing reaches the LLM without their explicit, conscious action, regardless of what state the app was in when a `warp://` URL was delivered.

## Goals

1. A Linear deeplink never sends a prompt to the model without an explicit user gesture from inside Warp.
2. When a Linear deeplink is opened, the user can always see the full prompt before choosing whether to send it.
3. The behavior is independent of whether the terminal was already in fullscreen agent view when the URL was handled.
4. Telemetry correctly reflects that Linear-originated prompts are not auto-submitted.

## Non-goals

- Closing the underlying unauthenticated `warp://` dispatch primitives (tracked in #655, #666). This spec assumes the dispatch primitive may be reachable by an attacker and hardens the Linear handler defensively.
- Redesigning the Linear deeplink feature itself (issue titles, branch names, richer metadata).
- Changing auto-submit behavior for other `AgentViewEntryOrigin` variants (e.g. `SlashCommand`, `AcceptedPromptSuggestion`, `Cli`). Those have different trust models and are out of scope here.
- Sanitizing or filtering the prompt string content. The mitigation relies on requiring a user gesture, not on content-based filtering.
- Removing the `prompt` query parameter from the Linear URI schema. The parameter remains supported; only the auto-submit behavior changes.

## Behavior

1. **Deeplink dispatch opens a new tab in agent view.** When `warp://linear/work?prompt=<PROMPT>` is dispatched, Warp opens a new tab, enters agent view for a new conversation (as today), and places the decoded `<PROMPT>` string into the agent input buffer for that conversation.

2. **The prompt is never submitted to the LLM automatically.** No code path initiated by a Linear deeplink submits the prompt on the user's behalf. Specifically:
   - The prompt is not auto-sent when the user was previously in fullscreen agent view in the focused terminal.
   - The prompt is not auto-sent when the user was not previously in agent view.
   - The prompt is not auto-sent on any platform (macOS, Linux, Windows) or via any dispatch primitive (OS URL handler, D-Bus, named pipe, third-party app opener).
   - The prompt is not auto-sent if the user has other agent conversations open or active elsewhere in the window.

3. **The user sees the full prompt before sending.** The attacker-supplied prompt is visible in the agent input buffer, rendered normally. The user can edit, clear, or reorder it before submitting. If the prompt contains newlines or very long content, the input renders it the same way it renders any user-typed multi-line prompt.

4. **"Enter again to send" affordance is shown.** When the Linear deeplink lands a prompt in the input buffer, Warp displays the existing ephemeral "press enter again to send to agent" confirmation message, signaling clearly that the prompt is pending and requires a deliberate keypress to send. The confirmation window matches the existing `ENTER_OR_EXIT_CONFIRMATION_WINDOW` timing.

5. **Explicit send gesture.** The prompt is submitted to the LLM only when the user performs an explicit send gesture in the agent input. That gesture is one of: pressing `Enter` while the ephemeral "enter again to send to agent" message from invariant 4 is still showing (within `ENTER_OR_EXIT_CONFIRMATION_WINDOW`); clicking the Send button in the agent input UI; or invoking the existing `SendInput` action via its bound keybinding (Cmd+Enter on macOS, Ctrl+Enter on Linux/Windows, as configured in the user's keymap). The gesture must target the agent input that was populated by the deeplink — not some other focused surface.
6. **Empty or missing prompt is a no-op for input.** If the URL has no `prompt` parameter, or the parameter decodes to an empty string, the agent input buffer is not overwritten. The tab still opens in agent view (matching current behavior for a prompt-less deeplink).
7. **No agent tools are invoked as a side effect of the deeplink.** Because nothing is submitted to the LLM until the user sends, none of the auto-executed agent tools (read_files, StartAgent, SendMessageToAgent, FetchConversation, UseComputer, etc.) are triggered by the act of opening the URL.
8. **Telemetry reflects non-auto-submission.** The `AgentViewEntered` telemetry event recorded for a Linear deeplink entry reports `did_auto_trigger_request = false`, regardless of prior agent view state. The `LinearIssueLinkOpened` event continues to fire on dispatch.
9. **Fallback display title is preserved.** The conversation's fallback display title remains `"Linear Issue"` so history and conversation pickers continue to identify the conversation as originating from a Linear deeplink.
10. **Regression guards for other origins.** Origins currently in `should_autotrigger_request`'s allowlist (e.g. `Input { was_prompt_autodetected: true }`, `SlashCommand` non-keybinding triggers, `Cli`, `AcceptedPromptSuggestion`) continue to auto-submit as before. The "already in fullscreen agent view" shortcut, which today promotes *any* origin to auto-submit, must no longer promote `LinearDeepLink` — and must not introduce regressions for other origins that previously relied on it.
11. **Background-window dispatch.** When Warp is backgrounded (or not the frontmost app) at the time the URL is dispatched, the new tab and its draft prompt must remain pending until the user foregrounds Warp. Nothing is submitted to the LLM during the time the user is not looking at the app. When the user does foreground Warp, they see the prompt in the input buffer and the "enter again to send" affordance exactly as in invariant 1–4; the pending state does not expire or auto-send itself on focus change.
12. **Logging and telemetry redaction.** The verbatim `prompt` query parameter from a Linear deeplink must not appear in log lines, telemetry payloads, error toasts, or conversation titles that escape the user's own machine. Specifically: the `AgentViewEntered` and `LinearIssueLinkOpened` telemetry events carry only the origin classifier and fixed schema fields — never the prompt body. Error logging (`log::error!`) emitted on a failed agent-view entry includes only the origin (`AgentViewEntryOrigin::LinearDeepLink`) and the structured error value, not the `initial_prompt`. Error toasts surface the error kind, not the attacker-supplied prompt. The input buffer is the only user-visible destination for the prompt content.
13. **Error paths remain silent to the user.** If entering agent view fails for a Linear deeplink, the existing error toast/logging path is used, subject to invariant 12's redaction rules.

## Success Criteria
1. Opening `warp://linear/work?prompt=<anything>` while the focused terminal is in fullscreen agent view results in the prompt appearing in the input buffer with an "enter again to send" confirmation — the prompt is not sent to the LLM.
2. Opening the same URL while the focused terminal is not in agent view results in identical behavior: new tab, agent view for a new conversation, prompt populated in the input buffer, confirmation shown, nothing sent.
3. Opening the same URL while Warp is backgrounded does not submit the prompt; on foregrounding, the user sees the draft and affordance and must act explicitly.
4. No LLM request, agent tool invocation, or model-facing network call is triggered by the act of handling the Linear deeplink, on any supported platform.
5. Telemetry for the Linear deeplink entry records `did_auto_trigger_request = false`, and no emitted telemetry, log line, toast, or conversation title contains the verbatim `prompt` body.
6. Non-Linear origins that were previously auto-submitted continue to auto-submit with the same timing as before the fix.
